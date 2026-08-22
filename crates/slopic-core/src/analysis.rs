use crate::ast::{self, Program};
use crate::diagnostic::{Diagnostic, Span};
use crate::sema::{self, BindingId, TExpr, TExprKind, TPattern, TypedFunction, TypedProgram};
use crate::syntax::{parse_lossless, LosslessSyntax, SyntaxKind, SyntaxToken};
use crate::{lexer, parser, reader, CompileOptions};
use serde::Serialize;
use std::collections::HashMap;

pub type SymbolId = u32;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnalysisSymbolKind {
    Function,
    Parameter,
    Variable,
    Struct,
    Enum,
    Constructor,
    Field,
    Builtin,
    Constant,
}

#[derive(Clone, Debug, Serialize)]
pub struct AnalysisSymbol {
    pub id: SymbolId,
    pub name: String,
    pub kind: AnalysisSymbolKind,
    pub detail: String,
    /// The sentence written above the declaration with `;;` (`D-134`).
    ///
    /// `detail` is what the compiler knows and this is what a person wrote, so
    /// they are two fields rather than one: a reader wants the type either way,
    /// and only some declarations carry prose.
    pub doc: Option<String>,
    pub definition: Span,
    pub scope: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct SymbolOccurrence {
    pub symbol: SymbolId,
    pub span: Span,
    pub is_definition: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Analysis {
    pub diagnostics: Vec<Diagnostic>,
    pub syntax: LosslessSyntax,
    pub program: Option<TypedProgram>,
    pub symbols: Vec<AnalysisSymbol>,
    pub occurrences: Vec<SymbolOccurrence>,
}

impl Analysis {
    pub fn symbol_at(&self, offset: usize) -> Option<&AnalysisSymbol> {
        let occurrence = self
            .occurrences
            .iter()
            .find(|item| item.span.start <= offset && offset < item.span.end)?;
        self.symbols
            .iter()
            .find(|symbol| symbol.id == occurrence.symbol)
    }

    pub fn visible_symbols(&self, offset: usize) -> Vec<&AnalysisSymbol> {
        self.symbols
            .iter()
            .filter(|symbol| {
                symbol.kind == AnalysisSymbolKind::Builtin
                    || (symbol.scope.start <= offset
                        && offset <= symbol.scope.end
                        && (matches!(
                            symbol.kind,
                            AnalysisSymbolKind::Function
                                | AnalysisSymbolKind::Struct
                                | AnalysisSymbolKind::Enum
                                | AnalysisSymbolKind::Constructor
                                | AnalysisSymbolKind::Constant
                        ) || symbol.definition.start <= offset))
            })
            .collect()
    }

    pub fn occurrences_of(&self, symbol: SymbolId) -> impl Iterator<Item = &SymbolOccurrence> {
        self.occurrences
            .iter()
            .filter(move |occurrence| occurrence.symbol == symbol)
    }
}

pub fn analyze_source(file: &str, source: &str, options: &CompileOptions) -> Analysis {
    let syntax = parse_lossless(source);
    let tokens = match lexer::lex(file, source) {
        Ok(tokens) => tokens,
        Err(diagnostics) => {
            return Analysis {
                diagnostics,
                syntax,
                program: None,
                symbols: Vec::new(),
                occurrences: Vec::new(),
            };
        }
    };
    let tokens = match reader::expand(file, &tokens) {
        Ok(tokens) => tokens,
        Err(diagnostics) => {
            return Analysis {
                diagnostics,
                syntax,
                program: None,
                symbols: Vec::new(),
                occurrences: Vec::new(),
            };
        }
    };
    let forms = match parser::parse(file, &tokens) {
        Ok(forms) => forms,
        Err(diagnostics) => {
            return Analysis {
                diagnostics,
                syntax,
                program: None,
                symbols: Vec::new(),
                occurrences: Vec::new(),
            };
        }
    };
    let mut ast = match ast::build_program(file, &forms) {
        Ok(program) => program,
        Err(diagnostics) => {
            return Analysis {
                diagnostics,
                syntax,
                program: None,
                symbols: Vec::new(),
                occurrences: Vec::new(),
            };
        }
    };
    // The editor shows the program that is being built, not every program the
    // file could be (`D-136`).
    ast::select_for_target(&mut ast, &options.target);
    let mut warnings = Vec::new();
    match sema::analyze_with_options(
        file,
        &ast,
        &options.language_items,
        options.validate_entry_point,
        &mut warnings,
    ) {
        Ok(program) => {
            let (symbols, occurrences) =
                SymbolIndexBuilder::new(source, &syntax.tokens, &ast).build(&program);
            // A warning leaves a compilation that succeeded, so `diagnostics`
            // is no longer empty exactly when `program` is `Some` (`D-122`).
            // Every caller already decides failure by the program rather than
            // by this list, which is what makes that safe.
            Analysis {
                diagnostics: warnings,
                syntax,
                program: Some(program),
                symbols,
                occurrences,
            }
        }
        Err(diagnostics) => Analysis {
            diagnostics,
            syntax,
            program: None,
            symbols: Vec::new(),
            occurrences: Vec::new(),
        },
    }
}

struct SymbolIndexBuilder<'a> {
    source: &'a str,
    syntax: &'a [SyntaxToken],
    ast: &'a Program,
    symbols: Vec<AnalysisSymbol>,
    occurrences: Vec<SymbolOccurrence>,
    top_level: HashMap<String, SymbolId>,
    fields: HashMap<(String, usize), SymbolId>,
    next_id: SymbolId,
}

impl<'a> SymbolIndexBuilder<'a> {
    fn new(source: &'a str, syntax: &'a [SyntaxToken], ast: &'a Program) -> Self {
        Self {
            source,
            syntax,
            ast,
            symbols: Vec::new(),
            occurrences: Vec::new(),
            top_level: HashMap::new(),
            fields: HashMap::new(),
            next_id: 0,
        }
    }

    fn build(mut self, program: &TypedProgram) -> (Vec<AnalysisSymbol>, Vec<SymbolOccurrence>) {
        let file_scope = Span {
            start: 0,
            end: self.source.len(),
            line: 1,
            column: 1,
        };
        for function in &self.ast.functions {
            self.define_top_level(
                &function.name,
                AnalysisSymbolKind::Function,
                format!("fn {} -> {}", function.name, function.return_type),
                function.span,
                file_scope,
            );
        }
        for structure in &self.ast.structs {
            self.define_top_level(
                &structure.name,
                AnalysisSymbolKind::Struct,
                format!("struct {}", structure.name),
                structure.span,
                file_scope,
            );
            for (index, field) in structure.fields.iter().enumerate() {
                let definition = self
                    .atom_span(field.span, &field.name)
                    .unwrap_or(field.span);
                let id = self.define(
                    &field.name,
                    AnalysisSymbolKind::Field,
                    format!("field {}.{}: {}", structure.name, field.name, field.ty),
                    definition,
                    file_scope,
                );
                self.fields.insert((structure.name.clone(), index), id);
            }
        }
        for enumeration in &self.ast.enums {
            self.define_top_level(
                &enumeration.name,
                AnalysisSymbolKind::Enum,
                format!("enum {}", enumeration.name),
                enumeration.span,
                file_scope,
            );
            for variant in &enumeration.variants {
                let path = format!("{}:{}", enumeration.name, variant.name);
                let definition = self
                    .atom_span(variant.span, &variant.name)
                    .unwrap_or(variant.span);
                let id = self.define(
                    &path,
                    AnalysisSymbolKind::Constructor,
                    format!("constructor {path}"),
                    definition,
                    file_scope,
                );
                self.top_level.insert(path, id);
            }
        }
        for constant in &self.ast.consts {
            let detail = match &constant.ty {
                Some(ty) => format!("const {} : {ty}", constant.name),
                None => format!("const {}", constant.name),
            };
            self.define_top_level(
                &constant.name,
                AnalysisSymbolKind::Constant,
                detail,
                constant.span,
                file_scope,
            );
        }
        for (name, detail) in BUILTINS {
            self.define_top_level(
                name,
                AnalysisSymbolKind::Builtin,
                (*detail).to_owned(),
                Span::default(),
                file_scope,
            );
        }
        for function in &program.functions {
            self.function(function);
        }
        (self.symbols, self.occurrences)
    }

    fn define_top_level(
        &mut self,
        name: &str,
        kind: AnalysisSymbolKind,
        detail: String,
        outer: Span,
        scope: Span,
    ) -> SymbolId {
        let definition = self.atom_span(outer, name).unwrap_or(outer);
        let id = self.define(name, kind, detail, definition, scope);
        self.attach_doc(id, outer);
        self.top_level.insert(name.to_owned(), id);
        id
    }

    fn define(
        &mut self,
        name: &str,
        kind: AnalysisSymbolKind,
        detail: String,
        definition: Span,
        scope: Span,
    ) -> SymbolId {
        let id = self.next_id;
        self.next_id += 1;
        self.symbols.push(AnalysisSymbol {
            id,
            name: name.to_owned(),
            kind,
            detail,
            doc: None,
            definition,
            scope,
        });
        if definition != Span::default() {
            self.occurrences.push(SymbolOccurrence {
                symbol: id,
                span: definition,
                is_definition: true,
            });
        }
        id
    }

    fn function(&mut self, function: &TypedFunction) {
        let mut bindings = HashMap::new();
        for parameter in &function.params {
            let definition = self
                .atom_span(parameter.span, &parameter.name)
                .unwrap_or(parameter.span);
            let id = self.define(
                &parameter.name,
                AnalysisSymbolKind::Parameter,
                parameter.ty.to_string(),
                definition,
                function.span,
            );
            bindings.insert(parameter.id, id);
        }
        self.expr(&function.body, function.span, &mut bindings);
    }

    fn expr(
        &mut self,
        expression: &TExpr,
        scope: Span,
        bindings: &mut HashMap<BindingId, SymbolId>,
    ) {
        match &expression.kind {
            TExprKind::Var(binding) | TExprKind::Borrow { id: binding, .. } => {
                self.binding_reference(*binding, expression.span, bindings);
            }
            TExprKind::Defer(body) => self.expr(body, scope, bindings),
            TExprKind::Let {
                id, name, value, ..
            } => {
                self.expr(value, scope, bindings);
                let definition = self
                    .atom_span(expression.span, name)
                    .unwrap_or(expression.span);
                let symbol = self.define(
                    name,
                    AnalysisSymbolKind::Variable,
                    value.ty.to_string(),
                    definition,
                    scope,
                );
                bindings.insert(*id, symbol);
            }
            TExprKind::Set { id, value } => {
                self.binding_reference(*id, expression.span, bindings);
                self.expr(value, scope, bindings);
            }
            TExprKind::Do(expressions) => {
                for item in expressions {
                    self.expr(item, scope, bindings);
                }
            }
            TExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                self.expr(condition, scope, bindings);
                let mut then_bindings = bindings.clone();
                self.expr(then_expr, then_expr.span, &mut then_bindings);
                let mut else_bindings = bindings.clone();
                self.expr(else_expr, else_expr.span, &mut else_bindings);
            }
            TExprKind::Loop { body } => {
                let mut loop_bindings = bindings.clone();
                self.expr(body, body.span, &mut loop_bindings);
            }
            TExprKind::While { condition, body } => {
                self.expr(condition, scope, bindings);
                let mut loop_bindings = bindings.clone();
                self.expr(body, body.span, &mut loop_bindings);
            }
            TExprKind::Match { value, arms } => {
                self.expr(value, scope, bindings);
                for arm in arms {
                    let mut arm_bindings = bindings.clone();
                    self.pattern(&arm.pattern, arm.span, &mut arm_bindings);
                    if let Some(guard) = &arm.guard {
                        self.expr(guard, arm.span, &mut arm_bindings);
                    }
                    self.expr(&arm.body, arm.span, &mut arm_bindings);
                }
            }
            // A `fn` named where a value is expected is a reference to that
            // function, so rename and go-to-definition must follow it: a
            // rename that misses one renames a program into a different one.
            TExprKind::FnRef { name, .. } => self.top_level_reference(name, expression.span),
            // The same reference, one word narrower: what C is handed is the
            // address of a function somebody wrote, and renaming it has to
            // reach here too (`D-124`).
            TExprKind::FnPointer { name } => self.top_level_reference(name, expression.span),
            // The value behind the borrow is an ordinary expression and every
            // name in it is an ordinary reference (`D-126`).
            TExprKind::BorrowTemporary { value } => self.expr(value, scope, bindings),
            TExprKind::CallValue { callee, args } => {
                self.binding_reference(*callee, expression.span, bindings);
                for argument in args {
                    self.expr(argument, scope, bindings);
                }
            }
            // A capture is written twice: once as a use of the binding outside,
            // which rename has to follow, and once as a definition inside,
            // which is the name the body uses. They share a spelling and are
            // two bindings, so both halves are recorded (`D-102`).
            TExprKind::Lambda {
                captures,
                params,
                body,
                ..
            } => {
                let mut inner = bindings.clone();
                for capture in captures {
                    self.binding_reference(capture.from, expression.span, bindings);
                    let definition = self
                        .atom_span(expression.span, &capture.name)
                        .unwrap_or(expression.span);
                    let symbol = self.define(
                        &capture.name,
                        AnalysisSymbolKind::Variable,
                        capture.ty.to_string(),
                        definition,
                        expression.span,
                    );
                    inner.insert(capture.id, symbol);
                }
                for param in params {
                    let definition = self
                        .atom_span(param.span, &param.name)
                        .unwrap_or(param.span);
                    let symbol = self.define(
                        &param.name,
                        AnalysisSymbolKind::Variable,
                        param.ty.to_string(),
                        definition,
                        expression.span,
                    );
                    inner.insert(param.id, symbol);
                }
                self.expr(body, body.span, &mut inner);
            }
            TExprKind::Call { callee, args } | TExprKind::GenericCall { callee, args, .. } => {
                self.top_level_reference(callee, expression.span);
                for argument in args {
                    self.expr(argument, scope, bindings);
                }
            }
            TExprKind::Try { value, .. } | TExprKind::Convert { value } => {
                self.expr(value, scope, bindings)
            }
            TExprKind::StructInit { name, fields } => {
                self.top_level_reference(name, expression.span);
                for field in fields {
                    self.expr(field, scope, bindings);
                }
            }
            TExprKind::EnumInit {
                enum_name,
                variant,
                fields,
                ..
            } => {
                self.top_level_reference(&format!("{enum_name}:{variant}"), expression.span);
                for field in fields {
                    self.expr(field, scope, bindings);
                }
            }
            TExprKind::Field {
                base,
                struct_name,
                index,
            } => {
                self.binding_reference(*base, expression.span, bindings);
                if let Some(symbol) = self.fields.get(&(struct_name.clone(), *index)).copied() {
                    let name = self.symbols[symbol as usize].name.clone();
                    let span = self
                        .atom_span(expression.span, &name)
                        .unwrap_or(expression.span);
                    self.occurrences.push(SymbolOccurrence {
                        symbol,
                        span,
                        is_definition: false,
                    });
                }
            }
            TExprKind::Break(value) => {
                if let Some(value) = value {
                    self.expr(value, scope, bindings);
                }
            }
            // A `const` use is a reference to the declaration, the way a `fn`
            // named as a value is (`D-121`), so hover and rename follow it
            // even though the literal is already here.
            TExprKind::Const { name, .. } => self.top_level_reference(name, expression.span),
            TExprKind::Unit
            | TExprKind::Bool(_)
            | TExprKind::Int(_)
            | TExprKind::Float(_)
            | TExprKind::String(_)
            | TExprKind::Continue => {}
        }
    }

    fn pattern(
        &mut self,
        pattern: &TPattern,
        scope: Span,
        bindings: &mut HashMap<BindingId, SymbolId>,
    ) {
        match pattern {
            TPattern::Binding(pattern) => {
                let definition = self.atom_span(scope, &pattern.name).unwrap_or(scope);
                let symbol = self.define(
                    &pattern.name,
                    AnalysisSymbolKind::Variable,
                    pattern.ty.to_string(),
                    definition,
                    scope,
                );
                bindings.insert(pattern.id, symbol);
            }
            TPattern::Enum {
                enum_name,
                variant,
                fields,
                ..
            } => {
                self.top_level_reference(&format!("{enum_name}:{variant}"), scope);
                for field in fields {
                    self.pattern(&field.pattern, scope, bindings);
                }
            }
            TPattern::Struct {
                struct_name,
                fields,
            } => {
                self.top_level_reference(struct_name, scope);
                for field in fields {
                    self.pattern(&field.pattern, scope, bindings);
                }
            }
            TPattern::Wildcard | TPattern::Bool(_) | TPattern::Int(_) => {}
        }
    }

    fn binding_reference(
        &mut self,
        binding: BindingId,
        outer: Span,
        bindings: &HashMap<BindingId, SymbolId>,
    ) {
        let Some(symbol) = bindings.get(&binding).copied() else {
            return;
        };
        let name = self.symbols[symbol as usize].name.clone();
        let span = self.atom_span(outer, &name).unwrap_or(outer);
        self.occurrences.push(SymbolOccurrence {
            symbol,
            span,
            is_definition: false,
        });
    }

    fn top_level_reference(&mut self, name: &str, outer: Span) {
        let Some(symbol) = self.top_level.get(name).copied() else {
            return;
        };
        let span = self.atom_span(outer, name).unwrap_or(outer);
        self.occurrences.push(SymbolOccurrence {
            symbol,
            span,
            is_definition: false,
        });
    }

    /// Attaches whatever `;;` block sits above `outer` to a symbol (`D-134`).
    fn attach_doc(&mut self, id: SymbolId, outer: Span) {
        let doc = self.doc_above(outer);
        if let Some(symbol) = self.symbols.get_mut(id as usize) {
            symbol.doc = doc;
        }
    }

    fn doc_above(&self, outer: Span) -> Option<String> {
        doc_comment(self.syntax, outer)
    }

    fn atom_span(&self, outer: Span, name: &str) -> Option<Span> {
        self.syntax.iter().find_map(|token| {
            (token.kind == SyntaxKind::Atom
                && token.text == name
                && token.span.start >= outer.start
                && token.span.end <= outer.end)
                .then_some(token.span)
        })
    }
}

/// The `;;` block written directly above `outer`, as one string (`D-134`).
///
/// Read out of the lossless tokens rather than out of the AST, which carries no
/// trivia: the two hold the same information, and taking it from here costs the
/// lexer nothing and the grammar nothing. Public because the language server
/// asks the same question of a declaration in another file, where it has the
/// tokens and no `Analysis`.
///
/// The block is the run of `;;` lines immediately above the declaration. A blank
/// line ends it, because a comment separated by one is about the file rather
/// than about what follows. So does a comment that is not the first thing on its
/// line: `(fn a ...) ;; note` belongs to the line it is on and not to whatever
/// is written next.
pub fn doc_comment(tokens: &[SyntaxToken], outer: Span) -> Option<String> {
    let mut index = tokens
        .iter()
        .position(|token| token.span.start >= outer.start)?;
    let mut lines = Vec::new();
    while index > 0 {
        index -= 1;
        let token = &tokens[index];
        match token.kind {
            SyntaxKind::Whitespace => {
                if token.text.bytes().filter(|byte| *byte == b'\n').count() > 1 {
                    break;
                }
            }
            SyntaxKind::Comment => {
                let starts_the_line = index == 0
                    || (tokens[index - 1].kind == SyntaxKind::Whitespace
                        && tokens[index - 1].text.contains('\n'));
                let Some(text) = token.text.strip_prefix(";;").filter(|_| starts_the_line) else {
                    break;
                };
                lines.push(text.trim());
            }
            _ => break,
        }
    }
    lines.reverse();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

pub const BUILTINS: &[(&str, &str)] = &[
    ("clone", "clone(T | &T) -> T"),
    ("list", "list(T...) -> List<T>"),
    ("array", "array(T...) -> Array<T, N>"),
    ("slice", "slice(&Collection<T>, i64, i64) -> Slice<T>"),
    ("len", "len(&List<T> | &String) -> i64"),
    ("push", "push(&mut List<T>, T) -> unit"),
    ("get", "get(&List<T>, i64) -> T"),
    ("get-ref", "get-ref(&List<T>, i64) -> &T"),
    ("pop", "pop(&mut List<T>) -> Option<T>"),
    ("remove", "remove(&mut List<T>, i64) -> T"),
    ("replace", "replace(&mut List<T>, i64, T) -> T"),
    (".", "field access"),
    ("+", "numeric addition"),
    ("-", "numeric subtraction, or negation with one operand"),
    ("*", "numeric multiplication"),
    ("/", "numeric division, truncated toward zero"),
    ("%", "integer remainder, truncated to agree with /"),
    ("<", "numeric comparison"),
    (">", "numeric comparison"),
    ("<=", "numeric comparison"),
    (">=", "numeric comparison"),
    ("=", "equality"),
    ("!=", "inequality"),
    ("not", "not(bool) -> bool"),
    ("and", "and(bool bool ...) -> bool, short-circuiting"),
    ("or", "or(bool bool ...) -> bool, short-circuiting"),
    ("bit-and", "bitwise and"),
    ("bit-or", "bitwise or"),
    ("bit-xor", "bitwise exclusive or"),
    ("bit-not", "bitwise complement"),
    (
        "shl",
        "left shift; traps on an amount the type has no room for",
    ),
    ("shr", "right shift, arithmetic on a signed type"),
    (
        "<<",
        "compose functions, right to left: ((<< f g) x) is (f (g x))",
    ),
    (
        ">>",
        "compose functions, left to right: ((>> f g) x) is (g (f x))",
    ),
    (
        "volatile-read",
        "volatile-read((Ptr T)) -> T; inside `unsafe`",
    ),
    (
        "volatile-write",
        "volatile-write((Ptr T) T) -> unit; inside `unsafe`",
    ),
    (
        "ptr-offset",
        "ptr-offset((Ptr T) u64) -> (Ptr T), scaled by T; inside `unsafe`",
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semantic_index_links_bindings_and_calls() {
        let source = "(fn add ((x i64)) -> i64 (+ x 1))\n(fn main () -> i64 (let n 1) (add n))";
        let analysis = analyze_source("test.slp", source, &CompileOptions::default());
        assert!(analysis.diagnostics.is_empty());
        let add = analysis
            .symbols
            .iter()
            .find(|symbol| symbol.name == "add")
            .unwrap();
        assert_eq!(analysis.occurrences_of(add.id).count(), 2);
        let n = analysis
            .symbols
            .iter()
            .find(|symbol| symbol.name == "n")
            .unwrap();
        assert_eq!(analysis.occurrences_of(n.id).count(), 2);
    }

    #[test]
    fn a_doc_block_reaches_its_declaration_and_stops_where_it_should() {
        let source = concat!(
            "; An ordinary comment says nothing about what follows.\n",
            ";; The first line.\n",
            ";; The second.\n",
            "(struct Point ((x i64)))\n",
            "\n",
            ";; A block a blank line above belongs to the file.\n",
            "\n",
            "(fn detached () -> i64 42)\n",
            "(fn attached () -> i64 (detached)) ;; a note about this line\n",
            "(fn after () -> i64 1)\n",
            "(fn main () -> i64 (+ (after) (+ (attached) (detached))))\n",
        );
        let analysis = analyze_source("test.slp", source, &CompileOptions::default());
        assert!(analysis.diagnostics.is_empty());
        let doc = |name: &str| {
            analysis
                .symbols
                .iter()
                .find(|symbol| symbol.name == name)
                .unwrap()
                .doc
                .clone()
        };

        // Both `;;` lines, joined, and the single-`;` line above them is not
        // part of the block.
        assert_eq!(
            doc("Point").as_deref(),
            Some("The first line.\nThe second.")
        );
        // A field is not a declaration and carries none of it.
        assert_eq!(doc("x"), None);
        // A blank line ends the block, so nothing reaches `detached`.
        assert_eq!(doc("detached"), None);
        // A comment sharing a line with code belongs to that line, not to the
        // declaration written after it.
        assert_eq!(doc("after"), None);
        assert_eq!(doc("attached"), None);
    }

    #[test]
    fn semantic_index_distinguishes_fields_and_types() {
        let source = "(struct Point ((x i64)))\n(fn main () -> i64 (let p (Point :x 42)) (. p x))";
        let analysis = analyze_source("test.slp", source, &CompileOptions::default());
        assert!(analysis.diagnostics.is_empty());
        let field = analysis
            .symbols
            .iter()
            .find(|symbol| symbol.name == "x" && symbol.kind == AnalysisSymbolKind::Field)
            .unwrap();
        assert_eq!(analysis.occurrences_of(field.id).count(), 2);
        let structure = analysis
            .symbols
            .iter()
            .find(|symbol| symbol.name == "Point" && symbol.kind == AnalysisSymbolKind::Struct)
            .unwrap();
        assert_eq!(analysis.occurrences_of(structure.id).count(), 2);
    }

    #[test]
    fn library_analysis_keeps_semantics_without_requiring_main() {
        let source = "(export answer)\n(fn answer () -> i64 42)\n";
        let strict = analyze_source("lib.slp", source, &CompileOptions::default());
        assert!(strict
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == crate::diagnostic::codes::ENTRY_POINT));

        let library = analyze_source(
            "lib.slp",
            source,
            &CompileOptions {
                validate_entry_point: false,
                ..CompileOptions::default()
            },
        );
        assert!(library.diagnostics.is_empty());
        assert!(library.program.is_some());
        assert!(library.symbols.iter().any(|symbol| symbol.name == "answer"));
    }
}
