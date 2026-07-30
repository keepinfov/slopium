use crossbeam_channel::Sender;
use lsp_server::{Connection, Message, Notification, Request, Response};
use lsp_types::request::{
    Completion, DocumentSymbolRequest, GotoDefinition, HoverRequest, References, Rename,
    Request as LspRequest, SemanticTokensFullRequest,
};
use serde_json::{json, Value};
use slopic_core::analysis::{analyze_source, Analysis, AnalysisSymbolKind};
use slopic_core::ast::{Expr, ExprKind, PatternKind, Program, Type};
use slopic_core::diagnostic::Span;
use slopic_core::package::{
    analyze_package, DeclarationKind, ModuleSummary, PackageInput, PackageSource,
};
use slopic_core::syntax::SyntaxKind;
use slopic_core::CompileOptions;
use slopium_manifest::manifest::Project;
use slopium_manifest::resolve::{resolve, Resolution};
use slopium_manifest::source::SourceId;
use slopium_manifest::std_library::{std_module_path, STD_MODULES};
use slopium_manifest::version::Version;
use slopium_manifest::workspace::{load_project, load_workspace};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

const DOCUMENT_SYMBOL: &str = DocumentSymbolRequest::METHOD;
const SEMANTIC_TOKENS_FULL: &str = SemanticTokensFullRequest::METHOD;
const COMPLETION: &str = Completion::METHOD;
const HOVER: &str = HoverRequest::METHOD;
const DEFINITION: &str = GotoDefinition::METHOD;
const REFERENCES: &str = References::METHOD;
const RENAME: &str = Rename::METHOD;

struct Document {
    version: i32,
    text: String,
    analysis: Analysis,
}

#[derive(Clone)]
struct WorkspaceFile {
    text: String,
}

#[derive(Clone)]
struct WorkspaceLocation {
    uri: String,
    span: Span,
}

struct WorkspaceSymbol {
    name: String,
    kind: AnalysisSymbolKind,
    detail: String,
    definition: WorkspaceLocation,
    occurrences: Vec<WorkspaceLocation>,
}

#[derive(Default)]
struct Workspace {
    files: HashMap<String, WorkspaceFile>,
    diagnostics: HashMap<String, Vec<slopic_core::diagnostic::Diagnostic>>,
    symbols: HashMap<String, WorkspaceSymbol>,
    visible: HashMap<String, Vec<(String, String)>>,
}

#[derive(Default)]
struct Server {
    documents: HashMap<String, Document>,
    workspace: Option<Workspace>,
}

fn main() -> Result<(), Box<dyn Error + Sync + Send>> {
    let (connection, threads) = Connection::stdio();
    let (initialize_id, _) = connection.initialize_start()?;
    connection.initialize_finish(
        initialize_id,
        json!({
            "capabilities": {
                "positionEncoding": "utf-16",
                "textDocumentSync": { "openClose": true, "change": 1, "save": { "includeText": true } },
                "hoverProvider": true,
                "completionProvider": { "triggerCharacters": [":", "&"] },
                "definitionProvider": true,
                "referencesProvider": true,
                "documentSymbolProvider": true,
                "renameProvider": true,
                "semanticTokensProvider": {
                    "legend": {
                        "tokenTypes": ["keyword", "function", "variable", "parameter", "property", "type", "enumMember"],
                        "tokenModifiers": ["declaration", "readonly"]
                    },
                    "full": true
                }
            },
            "serverInfo": { "name": "slopium-lsp", "version": env!("CARGO_PKG_VERSION") }
        }),
    )?;

    let mut server = Server::default();
    for message in &connection.receiver {
        match message {
            Message::Request(request) => {
                if connection.handle_shutdown(&request)? {
                    break;
                }
                server.handle_request(&connection.sender, request)?;
            }
            Message::Notification(notification) => {
                server.handle_notification(&connection.sender, notification)?;
            }
            Message::Response(_) => {}
        }
    }
    threads.join()?;
    Ok(())
}

impl Server {
    fn handle_notification(
        &mut self,
        sender: &Sender<Message>,
        notification: Notification,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        match notification.method.as_str() {
            "textDocument/didOpen" => {
                let uri = string_at(&notification.params, &["textDocument", "uri"])?;
                let text = string_at(&notification.params, &["textDocument", "text"])?;
                let version = i32_at(&notification.params, &["textDocument", "version"])?;
                self.update(uri.clone(), version, text);
                self.publish_open_documents(sender)?;
            }
            "textDocument/didChange" => {
                let uri = string_at(&notification.params, &["textDocument", "uri"])?;
                let version = i32_at(&notification.params, &["textDocument", "version"])?;
                let text = notification
                    .params
                    .pointer("/contentChanges")
                    .and_then(Value::as_array)
                    .and_then(|changes| changes.last())
                    .and_then(|change| change.get("text"))
                    .and_then(Value::as_str)
                    .ok_or("didChange does not contain full text")?
                    .to_owned();
                if self.update(uri.clone(), version, text) {
                    self.publish_open_documents(sender)?;
                }
            }
            "textDocument/didSave" => {
                let uri = string_at(&notification.params, &["textDocument", "uri"])?;
                if let Some(text) = notification.params.get("text").and_then(Value::as_str) {
                    let version = self
                        .documents
                        .get(&uri)
                        .map_or(0, |document| document.version);
                    self.update(uri.clone(), version, text.to_owned());
                }
                self.publish_open_documents(sender)?;
            }
            "textDocument/didClose" => {
                let uri = string_at(&notification.params, &["textDocument", "uri"])?;
                self.documents.remove(&uri);
                sender.send(Message::Notification(Notification::new(
                    "textDocument/publishDiagnostics".to_owned(),
                    json!({ "uri": uri, "diagnostics": [] }),
                )))?;
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_request(
        &self,
        sender: &Sender<Message>,
        request: Request,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        let result = match request.method.as_str() {
            DOCUMENT_SYMBOL => self.document_symbols(&request.params),
            SEMANTIC_TOKENS_FULL => self.semantic_tokens(&request.params),
            COMPLETION => self.completion(&request.params),
            HOVER => self.hover(&request.params),
            DEFINITION => self.definition(&request.params),
            REFERENCES => self.references(&request.params),
            RENAME => self.rename(&request.params),
            _ => {
                sender.send(Message::Response(Response::new_err(
                    request.id,
                    lsp_server::ErrorCode::MethodNotFound as i32,
                    format!("unsupported method `{}`", request.method),
                )))?;
                return Ok(());
            }
        };
        match result {
            Ok(value) => sender.send(Message::Response(Response::new_ok(request.id, value)))?,
            Err(message) => sender.send(Message::Response(Response::new_err(
                request.id,
                lsp_server::ErrorCode::InvalidParams as i32,
                message,
            )))?,
        }
        Ok(())
    }

    fn update(&mut self, uri: String, version: i32, text: String) -> bool {
        if self
            .documents
            .get(&uri)
            .is_some_and(|document| version < document.version)
        {
            return false;
        }
        let analysis = analyze_source(
            &uri,
            &text,
            &CompileOptions {
                validate_entry_point: document_requires_entry_point(&uri),
                ..CompileOptions::default()
            },
        );
        self.documents.insert(
            uri.clone(),
            Document {
                version,
                text,
                analysis,
            },
        );
        self.refresh_workspace(&uri);
        true
    }

    fn publish_open_documents(
        &self,
        sender: &Sender<Message>,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        for uri in self.documents.keys() {
            self.publish(sender, uri)?;
        }
        Ok(())
    }

    fn refresh_workspace(&mut self, uri: &str) {
        let Some(path) = file_uri_path(uri) else {
            self.workspace = None;
            return;
        };
        let Some(manifest) = find_manifest(&path) else {
            self.workspace = None;
            return;
        };
        self.workspace = build_workspace(&manifest, &self.documents).ok();
    }

    fn publish(
        &self,
        sender: &Sender<Message>,
        uri: &str,
    ) -> Result<(), Box<dyn Error + Sync + Send>> {
        let Some(document) = self.documents.get(uri) else {
            return Ok(());
        };
        let workspace_diagnostics = self
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.diagnostics.get(uri));
        let diagnostics = workspace_diagnostics
            .map(Vec::as_slice)
            .unwrap_or(&document.analysis.diagnostics)
            .iter()
            .map(|diagnostic| {
                let mut message = diagnostic.message.clone();
                if let Some(help) = &diagnostic.help {
                    message.push_str("\nhelp: ");
                    message.push_str(help);
                }
                for note in &diagnostic.notes {
                    message.push_str("\nnote: ");
                    message.push_str(note);
                }
                json!({
                    "range": span_range(&document.text, diagnostic.span),
                    "severity": match diagnostic.severity {
                        slopic_core::diagnostic::Severity::Error => 1,
                        slopic_core::diagnostic::Severity::Warning => 2,
                    },
                    "code": diagnostic.code,
                    "source": "slopic",
                    "message": message,
                    "relatedInformation": diagnostic.labels.iter().map(|label| json!({
                        "location": { "uri": uri, "range": span_range(&document.text, label.span) },
                        "message": label.message
                    })).collect::<Vec<_>>(),
                    "data": { "suggestions": diagnostic.suggestions }
                })
            })
            .collect::<Vec<_>>();
        sender.send(Message::Notification(Notification::new(
            "textDocument/publishDiagnostics".to_owned(),
            json!({
                "uri": uri,
                "version": document.version,
                "diagnostics": diagnostics
            }),
        )))?;
        Ok(())
    }

    fn document<'a>(&'a self, params: &'a Value) -> Result<(&'a str, &'a Document), String> {
        let uri = params
            .pointer("/textDocument/uri")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing textDocument.uri".to_owned())?;
        self.documents
            .get(uri)
            .map(|document| (uri, document))
            .ok_or_else(|| format!("document `{uri}` is not open"))
    }

    fn document_symbols(&self, params: &Value) -> Result<Value, String> {
        let (_, document) = self.document(params)?;
        Ok(Value::Array(
            document
                .analysis
                .symbols
                .iter()
                .filter(|symbol| symbol.definition != Span::default())
                .map(|symbol| {
                    let range = span_range(&document.text, symbol.definition);
                    json!({
                        "name": symbol.name,
                        "detail": symbol.detail,
                        "kind": symbol_kind(symbol.kind),
                        "range": range,
                        "selectionRange": range
                    })
                })
                .collect(),
        ))
    }

    fn semantic_tokens(&self, params: &Value) -> Result<Value, String> {
        let (_, document) = self.document(params)?;
        let mut tokens = Vec::<(usize, usize, u32, u32)>::new();
        for occurrence in &document.analysis.occurrences {
            let Some(symbol) = document
                .analysis
                .symbols
                .iter()
                .find(|symbol| symbol.id == occurrence.symbol)
            else {
                continue;
            };
            let token_type = match symbol.kind {
                AnalysisSymbolKind::Function | AnalysisSymbolKind::Builtin => 1,
                AnalysisSymbolKind::Variable => 2,
                AnalysisSymbolKind::Parameter => 3,
                AnalysisSymbolKind::Field => 4,
                AnalysisSymbolKind::Struct | AnalysisSymbolKind::Enum => 5,
                AnalysisSymbolKind::Constructor => 6,
            };
            let modifiers = if occurrence.is_definition { 1 } else { 0 };
            tokens.push((
                occurrence.span.start,
                occurrence.span.end,
                token_type,
                modifiers,
            ));
        }
        const KEYWORDS: &[&str] = &[
            "fn", "test", "struct", "enum", "let", "mut", "set", "if", "match", "do", "true",
            "false", "_", "unit", "bool", "i32", "i64", "f64", "String", "List", "Array", "Slice",
            "loop", "while", "break", "continue", "export", "take", "try",
        ];
        for token in &document.analysis.syntax.tokens {
            if token.kind == SyntaxKind::Atom && KEYWORDS.contains(&token.text.as_str()) {
                let token_type = if matches!(
                    token.text.as_str(),
                    "unit" | "bool" | "i32" | "i64" | "f64" | "String" | "List" | "Array" | "Slice"
                ) {
                    5
                } else {
                    0
                };
                tokens.push((token.span.start, token.span.end, token_type, 0));
            }
        }
        tokens.sort_by_key(|token| token.0);
        tokens.dedup_by_key(|token| token.0);
        let mut data = Vec::new();
        let mut previous_line = 0u32;
        let mut previous_start = 0u32;
        for (start, end, token_type, modifiers) in tokens {
            let position = offset_position(&document.text, start);
            let end_position = offset_position(&document.text, end);
            if position["line"] != end_position["line"] {
                continue;
            }
            let line = position["line"].as_u64().unwrap_or(0) as u32;
            let character = position["character"].as_u64().unwrap_or(0) as u32;
            let length = end_position["character"].as_u64().unwrap_or(0) as u32 - character;
            data.extend([
                line - previous_line,
                if line == previous_line {
                    character - previous_start
                } else {
                    character
                },
                length,
                token_type,
                modifiers,
            ]);
            previous_line = line;
            previous_start = character;
        }
        Ok(json!({ "data": data }))
    }

    fn completion(&self, params: &Value) -> Result<Value, String> {
        let (uri, document) = self.document(params)?;
        let offset = request_offset(params, &document.text)?;
        let mut seen = HashSet::new();
        let mut items = Vec::new();
        for symbol in document.analysis.visible_symbols(offset) {
            if seen.insert(symbol.name.clone()) {
                items.push(json!({
                    "label": symbol.name,
                    "kind": completion_kind(symbol.kind),
                    "detail": symbol.detail
                }));
            }
        }
        if let Some(workspace) = &self.workspace {
            for (label, canonical) in workspace.visible.get(uri).into_iter().flatten() {
                let Some(symbol) = workspace.symbols.get(canonical) else {
                    continue;
                };
                if seen.insert(label.clone()) {
                    items.push(json!({
                        "label": label,
                        "kind": completion_kind(symbol.kind),
                        "detail": symbol.detail
                    }));
                }
            }
        }
        for keyword in [
            "fn", "test", "struct", "enum", "export", "take", "let", "set", "if", "match", "do",
            "loop", "while", "break", "continue", "try", "&", "&mut",
        ] {
            if seen.insert(keyword.to_owned()) {
                items.push(json!({ "label": keyword, "kind": 14 }));
            }
        }
        Ok(json!({ "isIncomplete": false, "items": items }))
    }

    fn hover(&self, params: &Value) -> Result<Value, String> {
        let (uri, document) = self.document(params)?;
        let offset = request_offset(params, &document.text)?;
        if let Some(symbol) = self
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.symbol_at(uri, offset))
        {
            let mut hover = json!({
                "contents": { "kind": "markdown", "value": format!("`{}`\n\n{}", symbol.name, symbol.detail) },
            });
            if let Some(span) = workspace_span_for_uri(symbol, uri) {
                hover["range"] = span_range(&document.text, span);
            }
            return Ok(hover);
        }
        let Some(symbol) = document.analysis.symbol_at(offset) else {
            return Ok(Value::Null);
        };
        Ok(json!({
            "contents": { "kind": "markdown", "value": format!("`{}`\n\n{}", symbol.name, symbol.detail) },
            "range": span_range(&document.text, symbol.definition)
        }))
    }

    fn definition(&self, params: &Value) -> Result<Value, String> {
        let (uri, document) = self.document(params)?;
        let offset = request_offset(params, &document.text)?;
        if let Some(symbol) = self
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.symbol_at(uri, offset))
        {
            let text = self
                .workspace
                .as_ref()
                .and_then(|workspace| workspace.files.get(&symbol.definition.uri))
                .map_or("", |file| file.text.as_str());
            return Ok(json!({
                "uri": symbol.definition.uri,
                "range": span_range(text, symbol.definition.span)
            }));
        }
        let Some(symbol) = document.analysis.symbol_at(offset) else {
            return Ok(Value::Null);
        };
        if symbol.definition == Span::default() {
            return Ok(Value::Null);
        }
        Ok(json!({
            "uri": uri,
            "range": span_range(&document.text, symbol.definition)
        }))
    }

    fn references(&self, params: &Value) -> Result<Value, String> {
        let (uri, document) = self.document(params)?;
        let offset = request_offset(params, &document.text)?;
        let include_definition = params
            .pointer("/context/includeDeclaration")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        if let Some(symbol) = self
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.symbol_at(uri, offset))
        {
            let locations = symbol
                .occurrences
                .iter()
                .filter(|location| {
                    include_definition
                        || location.uri != symbol.definition.uri
                        || location.span != symbol.definition.span
                })
                .map(|location| {
                    let text = self
                        .workspace
                        .as_ref()
                        .and_then(|workspace| workspace.files.get(&location.uri))
                        .map_or("", |file| file.text.as_str());
                    json!({
                        "uri": location.uri,
                        "range": span_range(text, location.span)
                    })
                })
                .collect::<Vec<_>>();
            return Ok(Value::Array(locations));
        }
        let Some(symbol) = document.analysis.symbol_at(offset) else {
            return Ok(json!([]));
        };
        Ok(Value::Array(
            document
                .analysis
                .occurrences_of(symbol.id)
                .filter(|occurrence| include_definition || !occurrence.is_definition)
                .map(|occurrence| {
                    json!({ "uri": uri, "range": span_range(&document.text, occurrence.span) })
                })
                .collect(),
        ))
    }

    fn rename(&self, params: &Value) -> Result<Value, String> {
        let (uri, document) = self.document(params)?;
        let offset = request_offset(params, &document.text)?;
        let new_name = params
            .get("newName")
            .and_then(Value::as_str)
            .ok_or_else(|| "missing newName".to_owned())?;
        if new_name.is_empty()
            || new_name
                .chars()
                .any(|character| character.is_whitespace() || "()\";".contains(character))
        {
            return Err("newName is not a valid Slopium atom".to_owned());
        }
        if let Some(symbol) = self
            .workspace
            .as_ref()
            .and_then(|workspace| workspace.symbol_at(uri, offset))
        {
            let mut changes = serde_json::Map::new();
            for location in &symbol.occurrences {
                let text = self
                    .workspace
                    .as_ref()
                    .and_then(|workspace| workspace.files.get(&location.uri))
                    .map_or("", |file| file.text.as_str());
                changes
                    .entry(location.uri.clone())
                    .or_insert_with(|| Value::Array(Vec::new()))
                    .as_array_mut()
                    .expect("workspace edit array")
                    .push(json!({
                        "range": span_range(text, location.span),
                        "newText": new_name
                    }));
            }
            return Ok(json!({ "changes": changes }));
        }
        let Some(symbol) = document.analysis.symbol_at(offset) else {
            return Ok(Value::Null);
        };
        if symbol.kind == AnalysisSymbolKind::Builtin {
            return Err("builtins cannot be renamed".to_owned());
        }
        let edits = document
            .analysis
            .occurrences_of(symbol.id)
            .map(|occurrence| {
                json!({
                    "range": span_range(&document.text, occurrence.span),
                    "newText": new_name
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({ "changes": { uri: edits } }))
    }
}

impl Workspace {
    fn symbol_at(&self, uri: &str, offset: usize) -> Option<&WorkspaceSymbol> {
        self.symbols.values().find(|symbol| {
            symbol.occurrences.iter().any(|location| {
                location.uri == uri && location.span.start <= offset && offset < location.span.end
            })
        })
    }
}

/// The symbol's span *within `uri`*, or `None` when it has none. Falling back
/// to the definition would hand back an offset measured against a different
/// file, which produces a nonsensical range at best.
fn workspace_span_for_uri(symbol: &WorkspaceSymbol, uri: &str) -> Option<Span> {
    symbol
        .occurrences
        .iter()
        .find(|location| location.uri == uri)
        .map(|location| location.span)
        .or_else(|| (symbol.definition.uri == uri).then_some(symbol.definition.span))
}

/// Whether this package's entry module must define `main`.
///
/// `D-015`: a `lib.slp` entry and a manifest defining `[language-items]` are
/// both library packages, and neither needs an entry point.
fn validates_entry_point(project: &Project) -> bool {
    project.manifest.language_items.is_empty() && !project.is_library()
}

struct ParsedWorkspaceFile {
    uri: String,
    program: Program,
    tokens: Vec<slopic_core::lexer::Token>,
}

/// Analyze the package an open file belongs to.
///
/// The manifest passed in is the nearest one above the file, so in a workspace
/// that is the file's own member: a member is analyzed as itself, against its
/// own dependencies, rather than as part of one flat blob of every member's
/// sources. The workspace is still loaded around it, because a member manifest
/// may inherit fields that only the workspace knows.
fn build_workspace(
    manifest_path: &Path,
    open_documents: &HashMap<String, Document>,
) -> Result<Workspace, String> {
    let loaded = load_workspace(Some(manifest_path.to_path_buf()))?;
    let project = loaded.select(None, false)?[0].clone();
    let source_root = project.source_root()?;
    let entry = project
        .entry_path()
        .canonicalize()
        .map_err(|error| format!("cannot resolve package entry: {error}"))?;
    let entry_module = module_from_source_path(&source_root, &entry)?;
    let mut source_paths = Vec::new();
    collect_workspace_sources(&source_root, &mut source_paths)?;
    source_paths.sort();

    let mut files = HashMap::new();
    let mut sources = Vec::new();
    for path in source_paths {
        let canonical = path
            .canonicalize()
            .map_err(|error| format!("cannot resolve `{}`: {error}", path.display()))?;
        let uri = path_file_uri(&canonical);
        let text = open_documents
            .get(&uri)
            .map(|document| document.text.clone())
            .unwrap_or_else(|| fs::read_to_string(&canonical).unwrap_or_default());
        let module = module_from_source_path(&source_root, &canonical)?;
        files.insert(uri, WorkspaceFile { text: text.clone() });
        sources.push(PackageSource {
            path: canonical.display().to_string(),
            namespace: None,
            module,
            source: text,
        });
    }
    let toolchain_version = Version::parse(slopic_core::STANDARD_LIBRARY_VERSION)?;
    let resolution = resolve(&project, &loaded, &toolchain_version)?;
    let language_items = language_items_from(&resolution);
    add_resolved_dependencies(&resolution, open_documents, &mut files, &mut sources)?;
    let input = PackageInput {
        name: project.name.clone(),
        entry_module,
        files: sources,
    };
    let package = analyze_package(
        &input,
        &CompileOptions {
            language_items,
            validate_entry_point: validates_entry_point(&project),
            ..CompileOptions::default()
        },
    );
    let mut diagnostics = files
        .keys()
        .map(|uri| (uri.clone(), Vec::new()))
        .collect::<HashMap<_, _>>();
    for diagnostic in &package.diagnostics {
        let path = PathBuf::from(&diagnostic.file);
        let uri = if path.is_absolute() {
            path_file_uri(&path)
        } else {
            path_file_uri(&entry)
        };
        diagnostics.entry(uri).or_default().push(diagnostic.clone());
    }

    let mut parsed = HashMap::<String, ParsedWorkspaceFile>::new();
    for source in &input.files {
        let Ok(tokens) = slopic_core::lexer::lex(&source.path, &source.source) else {
            continue;
        };
        let Ok(forms) = slopic_core::parser::parse(&source.path, &tokens) else {
            continue;
        };
        let Ok(program) = slopic_core::ast::build_program(&source.path, &forms) else {
            continue;
        };
        let full_module = source.namespace.as_ref().map_or_else(
            || source.module.clone(),
            |namespace| format!("{namespace}:{}", source.module),
        );
        parsed.insert(
            full_module.clone(),
            ParsedWorkspaceFile {
                uri: path_file_uri(Path::new(&source.path)),
                program,
                tokens,
            },
        );
    }
    let mut workspace = Workspace {
        files,
        diagnostics,
        symbols: HashMap::new(),
        visible: HashMap::new(),
    };
    index_workspace_symbols(&mut workspace, &package.modules, &parsed);
    Ok(workspace)
}

/// Turn a resolved graph into the flat source list `analyze_package` wants.
///
/// This replaces a second recursive dependency walk that mirrored the project
/// manager's. Two walks meant the editor could namespace a module differently
/// from the build, and nothing compared them (`D-034`).
fn add_resolved_dependencies(
    resolution: &Resolution,
    open_documents: &HashMap<String, Document>,
    files: &mut HashMap<String, WorkspaceFile>,
    sources: &mut Vec<PackageSource>,
) -> Result<(), String> {
    for package in resolution.dependencies() {
        let namespace = package.namespace().to_owned();
        match (&package.id.source, &package.project) {
            (SourceId::Toolchain, _) => {
                for (module, source) in STD_MODULES {
                    sources.push(PackageSource {
                        path: std_module_path(module),
                        namespace: Some(namespace.clone()),
                        module: (*module).into(),
                        source: (*source).into(),
                    });
                }
            }
            (SourceId::Path(_), Some(project)) => {
                let source_root = project.source_root()?;
                let mut paths = Vec::new();
                collect_workspace_sources(&source_root, &mut paths)?;
                paths.sort();
                for path in paths {
                    let canonical = path
                        .canonicalize()
                        .map_err(|error| format!("cannot resolve `{}`: {error}", path.display()))?;
                    let uri = path_file_uri(&canonical);
                    let text = open_documents
                        .get(&uri)
                        .map(|document| document.text.clone())
                        .unwrap_or_else(|| fs::read_to_string(&canonical).unwrap_or_default());
                    let module = module_from_source_path(&source_root, &canonical)?;
                    files.insert(uri, WorkspaceFile { text: text.clone() });
                    sources.push(PackageSource {
                        path: canonical.display().to_string(),
                        namespace: Some(namespace.clone()),
                        module,
                        source: text,
                    });
                }
            }
            (SourceId::Path(path), None) => {
                return Err(format!(
                    "package `{}` at `{}` was resolved without a manifest",
                    package.id.name,
                    path.display()
                ))
            }
        }
    }
    Ok(())
}

/// The language items resolution settled on, in the compiler's shape.
fn language_items_from(resolution: &Resolution) -> slopic_core::LanguageItems {
    let mut items = slopic_core::LanguageItems::default();
    for (name, path) in &resolution.language_items {
        let slot = match name.as_str() {
            "option" => &mut items.option,
            "result" => &mut items.result,
            "result-ok" => &mut items.result_ok,
            "result-err" => &mut items.result_err,
            _ => continue,
        };
        *slot = Some(path.clone());
    }
    items
}

fn index_workspace_symbols(
    workspace: &mut Workspace,
    modules: &[ModuleSummary],
    parsed: &HashMap<String, ParsedWorkspaceFile>,
) {
    for module in modules {
        let uri = path_file_uri(Path::new(&module.path));
        let Some(file) = parsed.get(&module.module) else {
            continue;
        };
        for declaration in &module.declarations {
            let kind = match declaration.kind {
                DeclarationKind::Function => AnalysisSymbolKind::Function,
                DeclarationKind::Struct => AnalysisSymbolKind::Struct,
                DeclarationKind::Enum => AnalysisSymbolKind::Enum,
            };
            let definition = atom_span(&file.tokens, declaration.span, &declaration.name)
                .unwrap_or(declaration.span);
            let detail = declaration_detail(&file.program, &declaration.name, kind);
            let location = WorkspaceLocation {
                uri: uri.clone(),
                span: definition,
            };
            workspace.symbols.insert(
                declaration.canonical.clone(),
                WorkspaceSymbol {
                    name: declaration.name.clone(),
                    kind,
                    detail,
                    definition: location.clone(),
                    occurrences: vec![location],
                },
            );
        }
        for enumeration in &file.program.enums {
            let Some(declaration) = module
                .declarations
                .iter()
                .find(|declaration| declaration.name == enumeration.name)
            else {
                continue;
            };
            for variant in &enumeration.variants {
                let canonical = format!("{}:{}", declaration.canonical, variant.name);
                let definition =
                    atom_span(&file.tokens, variant.span, &variant.name).unwrap_or(variant.span);
                let location = WorkspaceLocation {
                    uri: uri.clone(),
                    span: definition,
                };
                workspace.symbols.insert(
                    canonical.clone(),
                    WorkspaceSymbol {
                        name: variant.name.clone(),
                        kind: AnalysisSymbolKind::Constructor,
                        detail: format!("constructor {}:{}", enumeration.name, variant.name),
                        definition: location.clone(),
                        occurrences: vec![location],
                    },
                );
            }
        }
    }

    for module in modules {
        let uri = path_file_uri(Path::new(&module.path));
        let visible = workspace.visible.entry(uri.clone()).or_default();
        for declaration in &module.declarations {
            visible.push((declaration.name.clone(), declaration.canonical.clone()));
        }
        for import in &module.imports {
            visible.push((import.name.clone(), import.canonical.clone()));
        }
        for exported in modules {
            for binding in &exported.export_bindings {
                visible.push((
                    format!("{}:{}", exported.module, binding.name),
                    binding.canonical.clone(),
                ));
            }
        }
        visible.sort();
        visible.dedup();

        let Some(file) = parsed.get(&module.module) else {
            continue;
        };
        scan_program_occurrences(workspace, modules, module, file);
    }
}

fn declaration_detail(program: &Program, name: &str, kind: AnalysisSymbolKind) -> String {
    match kind {
        AnalysisSymbolKind::Function => program
            .functions
            .iter()
            .find(|item| item.name == name)
            .map(|item| {
                let params = item
                    .params
                    .iter()
                    .map(|parameter| parameter.ty.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("fn {name}({params}) -> {}", item.return_type)
            })
            .unwrap_or_else(|| format!("fn {name}")),
        AnalysisSymbolKind::Struct => format!("struct {name}"),
        AnalysisSymbolKind::Enum => format!("enum {name}"),
        _ => name.to_owned(),
    }
}

fn scan_program_occurrences(
    workspace: &mut Workspace,
    modules: &[ModuleSummary],
    summary: &ModuleSummary,
    file: &ParsedWorkspaceFile,
) {
    for export in &file.program.exports {
        for item in &export.items {
            add_workspace_reference(workspace, modules, summary, file, &item.path, item.span);
        }
    }
    for take in &file.program.takes {
        for item in &take.items {
            if let Some(import) = summary
                .imports
                .iter()
                .find(|import| import.name == item.alias)
            {
                add_workspace_occurrence(
                    workspace,
                    &import.canonical,
                    &file.uri,
                    atom_span(&file.tokens, item.span, &item.path).unwrap_or(item.span),
                );
            }
        }
    }
    for function in &file.program.functions {
        for parameter in &function.params {
            scan_type_occurrences(
                workspace,
                modules,
                summary,
                file,
                &parameter.ty,
                parameter.span,
            );
        }
        scan_type_occurrences(
            workspace,
            modules,
            summary,
            file,
            &function.return_type,
            function.span,
        );
        scan_expr_occurrences(workspace, modules, summary, file, &function.body);
    }
    for structure in &file.program.structs {
        for field in &structure.fields {
            scan_type_occurrences(workspace, modules, summary, file, &field.ty, field.span);
        }
    }
    for enumeration in &file.program.enums {
        for variant in &enumeration.variants {
            for field in &variant.fields {
                scan_type_occurrences(workspace, modules, summary, file, &field.ty, field.span);
            }
        }
    }
    for test in &file.program.tests {
        scan_expr_occurrences(workspace, modules, summary, file, &test.body);
    }
}

fn scan_expr_occurrences(
    workspace: &mut Workspace,
    modules: &[ModuleSummary],
    summary: &ModuleSummary,
    file: &ParsedWorkspaceFile,
    expression: &Expr,
) {
    match &expression.kind {
        ExprKind::Let { value, .. } | ExprKind::Set { value, .. } => {
            scan_expr_occurrences(workspace, modules, summary, file, value);
        }
        ExprKind::Do(items) => {
            for item in items {
                scan_expr_occurrences(workspace, modules, summary, file, item);
            }
        }
        ExprKind::If {
            condition,
            then_expr,
            else_expr,
        } => {
            scan_expr_occurrences(workspace, modules, summary, file, condition);
            scan_expr_occurrences(workspace, modules, summary, file, then_expr);
            scan_expr_occurrences(workspace, modules, summary, file, else_expr);
        }
        ExprKind::Loop { body } => {
            scan_expr_occurrences(workspace, modules, summary, file, body);
        }
        ExprKind::While { condition, body } => {
            scan_expr_occurrences(workspace, modules, summary, file, condition);
            scan_expr_occurrences(workspace, modules, summary, file, body);
        }
        ExprKind::Match { value, arms } => {
            scan_expr_occurrences(workspace, modules, summary, file, value);
            for arm in arms {
                scan_pattern_occurrences(workspace, modules, summary, file, &arm.pattern);
                scan_expr_occurrences(workspace, modules, summary, file, &arm.body);
            }
        }
        ExprKind::Borrow { value, .. } | ExprKind::Try(value) => {
            scan_expr_occurrences(workspace, modules, summary, file, value);
        }
        ExprKind::Call { callee, args } => {
            add_workspace_reference(workspace, modules, summary, file, callee, expression.span);
            for argument in args {
                scan_expr_occurrences(workspace, modules, summary, file, argument);
            }
        }
        ExprKind::Unit
        | ExprKind::Bool(_)
        | ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::String(_)
        | ExprKind::Var(_)
        | ExprKind::Break
        | ExprKind::Continue => {}
    }
}

fn scan_pattern_occurrences(
    workspace: &mut Workspace,
    modules: &[ModuleSummary],
    summary: &ModuleSummary,
    file: &ParsedWorkspaceFile,
    pattern: &slopic_core::ast::Pattern,
) {
    match &pattern.kind {
        PatternKind::Enum { path, fields } => {
            add_workspace_reference(workspace, modules, summary, file, path, pattern.span);
            for field in fields {
                scan_pattern_occurrences(workspace, modules, summary, file, field);
            }
        }
        PatternKind::Struct { path, fields } => {
            add_workspace_reference(workspace, modules, summary, file, path, pattern.span);
            for (_, field) in fields {
                scan_pattern_occurrences(workspace, modules, summary, file, field);
            }
        }
        _ => {}
    }
}

fn scan_type_occurrences(
    workspace: &mut Workspace,
    modules: &[ModuleSummary],
    summary: &ModuleSummary,
    file: &ParsedWorkspaceFile,
    type_: &Type,
    span: Span,
) {
    match type_ {
        Type::Named(name) => {
            add_workspace_reference(workspace, modules, summary, file, name, span);
        }
        Type::Apply { name, args } => {
            add_workspace_reference(workspace, modules, summary, file, name, span);
            for argument in args {
                scan_type_occurrences(workspace, modules, summary, file, argument, span);
            }
        }
        Type::List(inner) | Type::Slice(inner) | Type::Ref { inner, .. } => {
            scan_type_occurrences(workspace, modules, summary, file, inner, span);
        }
        Type::Array { element, .. } => {
            scan_type_occurrences(workspace, modules, summary, file, element, span);
        }
        _ => {}
    }
}

fn add_workspace_reference(
    workspace: &mut Workspace,
    modules: &[ModuleSummary],
    summary: &ModuleSummary,
    file: &ParsedWorkspaceFile,
    name: &str,
    outer: Span,
) {
    let Some(canonical) = resolve_workspace_name(modules, summary, name) else {
        return;
    };
    let span = atom_span(&file.tokens, outer, name).unwrap_or(outer);
    add_workspace_occurrence(workspace, &canonical, &file.uri, span);
}

fn add_workspace_occurrence(workspace: &mut Workspace, canonical: &str, uri: &str, span: Span) {
    let Some(symbol) = workspace.symbols.get_mut(canonical) else {
        return;
    };
    if !symbol
        .occurrences
        .iter()
        .any(|location| location.uri == uri && location.span == span)
    {
        symbol.occurrences.push(WorkspaceLocation {
            uri: uri.to_owned(),
            span,
        });
    }
}

fn resolve_workspace_name(
    modules: &[ModuleSummary],
    current: &ModuleSummary,
    name: &str,
) -> Option<String> {
    if let Some(declaration) = current
        .declarations
        .iter()
        .find(|declaration| declaration.name == name)
    {
        return Some(declaration.canonical.clone());
    }
    if let Some(import) = current.imports.iter().find(|import| import.name == name) {
        return Some(import.canonical.clone());
    }
    if let Some((head, tail)) = name.split_once(':') {
        if let Some(declaration) = current
            .declarations
            .iter()
            .find(|declaration| declaration.name == head)
        {
            return Some(format!("{}:{tail}", declaration.canonical));
        }
        if let Some(import) = current.imports.iter().find(|import| import.name == head) {
            return Some(format!("{}:{tail}", import.canonical));
        }
    }
    let module = modules
        .iter()
        .filter(|module| {
            name.len() > module.module.len()
                && name.starts_with(&module.module)
                && name.as_bytes().get(module.module.len()) == Some(&b':')
        })
        .max_by_key(|module| module.module.len())?;
    let remainder = name.strip_prefix(&module.module)?.strip_prefix(':')?;
    let (exported, suffix) = remainder
        .split_once(':')
        .map_or((remainder, None), |(head, tail)| (head, Some(tail)));
    let canonical = module
        .export_bindings
        .iter()
        .find(|binding| binding.name == exported)?
        .canonical
        .clone();
    Some(suffix.map_or(canonical.clone(), |suffix| format!("{canonical}:{suffix}")))
}

fn atom_span(tokens: &[slopic_core::lexer::Token], outer: Span, name: &str) -> Option<Span> {
    tokens.iter().find_map(|token| {
        (matches!(&token.kind, slopic_core::lexer::TokenKind::Atom(text) if text == name)
            && token.span.start >= outer.start
            && token.span.end <= outer.end)
            .then_some(token.span)
    })
}

fn find_manifest(path: &Path) -> Option<PathBuf> {
    let start = if path.is_dir() { path } else { path.parent()? };
    start
        .ancestors()
        .map(|directory| directory.join("Slopium.toml"))
        .find(|candidate| candidate.is_file())
}

fn document_requires_entry_point(uri: &str) -> bool {
    let Some(path) = file_uri_path(uri) else {
        return true;
    };
    let Some(manifest_path) = find_manifest(&path) else {
        return true;
    };
    let Ok(project) = load_project(Some(manifest_path)) else {
        return true;
    };
    if !validates_entry_point(&project) {
        return false;
    }
    let entry = project.entry_path();
    match (path.canonicalize(), entry.canonicalize()) {
        (Ok(path), Ok(entry)) => path == entry,
        _ => path == entry,
    }
}

fn collect_workspace_sources(directory: &Path, output: &mut Vec<PathBuf>) -> Result<(), String> {
    slopic_core::collect_slp_sources(directory, output)
        .map_err(|error| format!("cannot read `{}`: {error}", directory.display()))
}

fn module_from_source_path(root: &Path, path: &Path) -> Result<String, String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|error| format!("cannot derive module for `{}`: {error}", path.display()))?;
    let mut parts = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let last = parts
        .last_mut()
        .ok_or_else(|| format!("invalid source path `{}`", path.display()))?;
    *last = Path::new(last)
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid source file name `{}`", path.display()))?
        .to_owned();
    Ok(parts.join(":"))
}

fn path_file_uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

fn file_uri_path(uri: &str) -> Option<PathBuf> {
    let encoded = uri.strip_prefix("file://")?;
    let mut bytes = Vec::with_capacity(encoded.len());
    let input = encoded.as_bytes();
    let mut index = 0;
    while index < input.len() {
        if input[index] == b'%' && index + 2 < input.len() {
            let hex = std::str::from_utf8(&input[index + 1..index + 3]).ok()?;
            bytes.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            bytes.push(input[index]);
            index += 1;
        }
    }
    String::from_utf8(bytes).ok().map(PathBuf::from)
}

fn string_at(value: &Value, path: &[&str]) -> Result<String, Box<dyn Error + Sync + Send>> {
    let mut value = value;
    for segment in path {
        value = value.get(*segment).ok_or("missing LSP field")?;
    }
    Ok(value
        .as_str()
        .ok_or("LSP field is not a string")?
        .to_owned())
}

fn i32_at(value: &Value, path: &[&str]) -> Result<i32, Box<dyn Error + Sync + Send>> {
    let mut value = value;
    for segment in path {
        value = value.get(*segment).ok_or("missing LSP field")?;
    }
    i32::try_from(value.as_i64().ok_or("LSP field is not an integer")?)
        .map_err(|error| error.into())
}

fn request_offset(params: &Value, text: &str) -> Result<usize, String> {
    let line = params
        .pointer("/position/line")
        .and_then(Value::as_u64)
        .ok_or_else(|| "missing position.line".to_owned())? as u32;
    let character = params
        .pointer("/position/character")
        .and_then(Value::as_u64)
        .ok_or_else(|| "missing position.character".to_owned())? as u32;
    position_offset(text, line, character).ok_or_else(|| "position is outside document".to_owned())
}

fn position_offset(text: &str, target_line: u32, target_character: u32) -> Option<usize> {
    let mut line = 0u32;
    let mut line_start = 0usize;
    for (index, character) in text.char_indices() {
        if line == target_line {
            let mut utf16 = 0u32;
            for (relative, value) in text[line_start..].char_indices() {
                if value == '\n' {
                    break;
                }
                if utf16 >= target_character {
                    return (utf16 == target_character).then_some(line_start + relative);
                }
                utf16 += value.len_utf16() as u32;
            }
            return (utf16 == target_character).then_some(
                text[line_start..]
                    .find('\n')
                    .map_or(text.len(), |relative| line_start + relative),
            );
        }
        if character == '\n' {
            line += 1;
            line_start = index + 1;
        }
    }
    (line == target_line && target_character == 0).then_some(line_start)
}

fn offset_position(text: &str, offset: usize) -> Value {
    // Spans can arrive from a different file than `text` (multi-module
    // packages shift offsets, and symbol lookups fall back to a definition in
    // another module), so clamping to the length is not enough: the offset has
    // to be walked back onto a character boundary or the slice panics and
    // takes the server down.
    let mut offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let prefix = &text[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() as u32;
    let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
    let character = text[line_start..offset]
        .chars()
        .map(|character| character.len_utf16() as u32)
        .sum::<u32>();
    json!({ "line": line, "character": character })
}

fn span_range(text: &str, span: Span) -> Value {
    json!({
        "start": offset_position(text, span.start),
        "end": offset_position(text, span.end.max(span.start))
    })
}

fn symbol_kind(kind: AnalysisSymbolKind) -> u32 {
    match kind {
        AnalysisSymbolKind::Function | AnalysisSymbolKind::Builtin => 12,
        AnalysisSymbolKind::Parameter | AnalysisSymbolKind::Variable => 13,
        AnalysisSymbolKind::Struct => 23,
        AnalysisSymbolKind::Enum => 10,
        AnalysisSymbolKind::Constructor => 22,
        AnalysisSymbolKind::Field => 8,
    }
}

fn completion_kind(kind: AnalysisSymbolKind) -> u32 {
    match kind {
        AnalysisSymbolKind::Function | AnalysisSymbolKind::Builtin => 3,
        AnalysisSymbolKind::Parameter | AnalysisSymbolKind::Variable => 6,
        AnalysisSymbolKind::Struct | AnalysisSymbolKind::Enum => 7,
        AnalysisSymbolKind::Constructor => 4,
        AnalysisSymbolKind::Field => 5,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static NEXT_WORKSPACE: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn utf16_positions_round_trip_unicode() {
        let source = "a💥b\nж";
        for offset in [0, 1, 5, 6, 7, 9] {
            let position = offset_position(source, offset);
            let line = position["line"].as_u64().unwrap() as u32;
            let character = position["character"].as_u64().unwrap() as u32;
            assert_eq!(position_offset(source, line, character), Some(offset));
        }
    }

    #[test]
    fn stale_document_version_is_ignored() {
        let mut server = Server::default();
        assert!(server.update("file:///test.slp".into(), 2, "(fn main () -> i32 0)".into()));
        assert!(!server.update("file:///test.slp".into(), 1, "broken".into()));
        assert_eq!(server.documents["file:///test.slp"].version, 2);
    }

    #[test]
    fn unsaved_document_supports_completion_navigation_and_rename() {
        let uri = "file:///test.slp";
        let source = "(fn main () -> i64 (let n 41) (+ n 1))";
        let mut server = Server::default();
        assert!(server.update(uri.into(), 7, source.into()));
        let reference = source.rfind("n 1").unwrap();
        let position = offset_position(source, reference);
        let params = json!({ "textDocument": { "uri": uri }, "position": position });

        let hover = server.hover(&params).unwrap();
        assert!(hover["contents"]["value"].as_str().unwrap().contains("i64"));
        let definition = server.definition(&params).unwrap();
        assert_ne!(definition, Value::Null);
        let completion = server.completion(&params).unwrap();
        assert!(completion["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["label"] == "n"));
        let rename = server
            .rename(&json!({
                "textDocument": { "uri": uri },
                "position": position,
                "newName": "answer"
            }))
            .unwrap();
        assert_eq!(rename["changes"][uri].as_array().unwrap().len(), 2);
    }

    #[test]
    fn workspace_links_imports_across_files_and_uses_unsaved_text() {
        let id = NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("slopium-lsp-workspace-{}-{id}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Slopium.toml"),
            "[package]\nname = \"workspace\"\nversion = \"0.2.0\"\nentry = \"src/main.slp\"\nsource = \"src\"\n\n[dependencies]\nstd = { toolchain = true }\n",
        )
        .unwrap();
        let geometry = "(export distance)\n(fn distance ((n i64)) -> i64 (+ n 1))\n";
        let main = "(take geometry (distance :as length))\n(fn main () -> i32 (do (println (length 41)) 0))\n";
        let geometry_path = root.join("src/geometry.slp");
        let main_path = root.join("src/main.slp");
        fs::write(&geometry_path, geometry).unwrap();
        fs::write(&main_path, main).unwrap();
        let geometry_uri = path_file_uri(&geometry_path.canonicalize().unwrap());
        let main_uri = path_file_uri(&main_path.canonicalize().unwrap());

        assert!(!document_requires_entry_point(&geometry_uri));
        assert!(document_requires_entry_point(&main_uri));
        let mut server = Server::default();
        assert!(server.update(geometry_uri.clone(), 1, geometry.into()));
        assert!(server.documents[&geometry_uri].analysis.program.is_some());
        assert!(server.update(main_uri.clone(), 1, main.into()));
        assert!(server
            .workspace
            .as_ref()
            .unwrap()
            .diagnostics
            .get(&main_uri)
            .unwrap()
            .is_empty());

        let call = main.rfind("length 41").unwrap();
        let params = json!({
            "textDocument": { "uri": main_uri },
            "position": offset_position(main, call)
        });
        let definition = server.definition(&params).unwrap();
        assert_eq!(definition["uri"], geometry_uri);
        let references = server
            .references(&json!({
                "textDocument": { "uri": main_uri },
                "position": offset_position(main, call),
                "context": { "includeDeclaration": true }
            }))
            .unwrap();
        assert!(references.as_array().unwrap().len() >= 3);
        let completion = server.completion(&params).unwrap();
        assert!(completion["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["label"] == "length"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn path_dependency_library_does_not_require_main() {
        let id = NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "slopium-lsp-foundation-{}-{id}",
            std::process::id()
        ));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Slopium.toml"),
            "[package]\nname = \"foundation\"\nversion = \"0.2.4\"\nentry = \"src/lib.slp\"\nsource = \"src\"\n",
        )
        .unwrap();
        fs::write(
            root.join("src/numbers.slp"),
            "(export forty)\n(fn forty () -> i64 40)\n",
        )
        .unwrap();
        let library = "(export (numbers:forty :as forty))\n";
        let library_path = root.join("src/lib.slp");
        fs::write(&library_path, library).unwrap();
        let library_path = library_path.canonicalize().unwrap();
        let library_uri = path_file_uri(&library_path);

        assert!(!document_requires_entry_point(&library_uri));
        let mut server = Server::default();
        assert!(server.update(library_uri.clone(), 1, library.into()));
        assert!(server.documents[&library_uri].analysis.program.is_some());

        let (sender, receiver) = crossbeam_channel::unbounded();
        server.publish(&sender, &library_uri).unwrap();
        let Message::Notification(published) = receiver.recv().unwrap() else {
            panic!("expected published diagnostics notification");
        };
        assert_eq!(published.method, "textDocument/publishDiagnostics");
        assert!(published.params["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn custom_standard_library_does_not_require_main() {
        let id = NEXT_WORKSPACE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "slopium-lsp-custom-std-{}-{id}",
            std::process::id()
        ));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Slopium.toml"),
            "[package]\nname = \"custom-std\"\nversion = \"0.2.4\"\nentry = \"src/lib.slp\"\nsource = \"src\"\n\n[language-items]\noption = \"lib:Option\"\nresult = \"lib:Result\"\nresult-ok = \"lib:Ok\"\nresult-err = \"lib:Err\"\n",
        )
        .unwrap();
        let library = "(export Option Result (Result:Ok :as Ok) (Result:Err :as Err))\n\
                       (enum Option (T) None (Some ((value T))))\n\
                       (enum Result (T E) (Ok ((value T))) (Err ((error E))))\n";
        let library_path = root.join("src/lib.slp");
        fs::write(&library_path, library).unwrap();
        let library_uri = path_file_uri(&library_path.canonicalize().unwrap());

        assert!(!document_requires_entry_point(&library_uri));
        let mut server = Server::default();
        assert!(server.update(library_uri.clone(), 1, library.into()));
        let document = &server.documents[&library_uri];
        assert!(document.analysis.program.is_some());
        assert!(!document
            .analysis
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == slopic_core::diagnostic::codes::ENTRY_POINT));
        assert!(
            !server.workspace.as_ref().unwrap().diagnostics[&library_uri]
                .iter()
                .any(|diagnostic| diagnostic.code == slopic_core::diagnostic::codes::ENTRY_POINT)
        );
        let (sender, receiver) = crossbeam_channel::unbounded();
        server.publish(&sender, &library_uri).unwrap();
        let Message::Notification(published) = receiver.recv().unwrap() else {
            panic!("expected published diagnostics notification");
        };
        assert_eq!(published.method, "textDocument/publishDiagnostics");
        assert!(published.params["diagnostics"]
            .as_array()
            .unwrap()
            .is_empty());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn offsets_inside_a_character_do_not_panic() {
        // Spans can come from another file in the package, so an offset may
        // land in the middle of a multi-byte character in *this* document.
        // Slicing there used to panic and take the server down.
        let text = "(println \"привет\")\nlet ключ 1\n";
        for offset in 0..text.len() + 8 {
            let position = offset_position(text, offset);
            assert!(position["line"].is_number());
            assert!(position["character"].is_number());
        }
        let span = Span {
            start: 11,
            end: 12,
            line: 1,
            column: 1,
        };
        assert!(!text.is_char_boundary(span.start));
        let _ = span_range(text, span);
    }

    #[test]
    fn a_symbol_absent_from_a_file_has_no_range_in_it() {
        let symbol = WorkspaceSymbol {
            name: "helper".into(),
            detail: String::new(),
            kind: AnalysisSymbolKind::Function,
            definition: WorkspaceLocation {
                uri: "file:///a.slp".into(),
                span: Span {
                    start: 40,
                    end: 46,
                    line: 3,
                    column: 1,
                },
            },
            occurrences: Vec::new(),
        };
        assert!(workspace_span_for_uri(&symbol, "file:///b.slp").is_none());
        assert!(workspace_span_for_uri(&symbol, "file:///a.slp").is_some());
    }
}
