use crate::ast::{self, Expr, ExprKind, ImportItem, MatchArm, Pattern, Program, TakeDecl, Type};
use crate::diagnostic::{codes, CompileResult, Diagnostic, SourceMap, Span};
use crate::sema::{self, TypedProgram};
use crate::{lexer, parser, reader, CompileOptions};
use serde::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct PackageSource {
    pub path: String,
    pub namespace: Option<String>,
    pub module: String,
    pub source: String,
}

#[derive(Clone, Debug)]
pub struct PackageInput {
    pub name: String,
    pub entry_module: String,
    pub files: Vec<PackageSource>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModuleSummary {
    pub path: String,
    pub module: String,
    pub exports: Vec<String>,
    pub export_bindings: Vec<ModuleBinding>,
    pub imports: Vec<ModuleBinding>,
    pub declarations: Vec<ModuleDeclaration>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModuleBinding {
    pub name: String,
    pub canonical: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct ModuleDeclaration {
    pub name: String,
    pub canonical: String,
    pub kind: DeclarationKind,
    pub span: Span,
}

#[derive(Clone, Debug, Serialize)]
pub struct PackageAnalysis {
    pub diagnostics: Vec<Diagnostic>,
    pub program: Option<TypedProgram>,
    pub modules: Vec<ModuleSummary>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum DeclarationKind {
    Function,
    Struct,
    Enum,
    Extern,
    Const,
}

#[derive(Clone, Debug)]
struct Decl {
    canonical: String,
    kind: DeclarationKind,
    span: Span,
}

struct ModuleUnit {
    path: String,
    module: String,
    namespace: Option<String>,
    source_len: usize,
    base: usize,
    program: Program,
    declarations: HashMap<String, Decl>,
    imports: HashMap<String, String>,
}

/// Where each source starts in the virtual concatenation that package analysis
/// merges the modules into.
///
/// Spans in the merged program are offsets into that concatenation. Both the
/// diagnostic remapper and the debug-line emitter turn one back into a file, so
/// the rule they share lives here rather than in either of them. The extra byte
/// per file keeps the ranges from touching, so a span at the very start of a
/// module cannot be read as the end of the one before it.
pub fn source_bases(files: &[PackageSource]) -> Vec<usize> {
    let mut base = 0;
    files
        .iter()
        .map(|source| {
            let start = base;
            base += source.source.len() + 1;
            start
        })
        .collect()
}

/// The file each span of a merged package belongs to.
pub fn source_map(input: &PackageInput) -> SourceMap {
    SourceMap::new(
        source_bases(&input.files)
            .into_iter()
            .zip(input.files.iter().map(|source| source.path.clone())),
    )
}

pub fn analyze_package(input: &PackageInput, options: &CompileOptions) -> PackageAnalysis {
    let mut diagnostics = Vec::new();
    let mut units = Vec::new();
    let mut seen_modules = HashMap::<String, String>::new();

    for source in &input.files {
        let full_module = source.namespace.as_ref().map_or_else(
            || source.module.clone(),
            |namespace| format!("{namespace}:{}", source.module),
        );
        if !valid_module_name(&full_module) {
            diagnostics.push(Diagnostic::error(
                codes::MODULE,
                &source.path,
                Span::default(),
                format!("invalid module identity `{full_module}`"),
            ));
            continue;
        }
        if let Some(previous) = seen_modules.insert(full_module.clone(), source.path.clone()) {
            diagnostics.push(
                Diagnostic::error(
                    codes::MODULE,
                    &source.path,
                    Span::default(),
                    format!("module `{full_module}` is defined by more than one file"),
                )
                .with_note(format!("the other source is `{previous}`")),
            );
            continue;
        }
        let tokens = match lexer::lex(&source.path, &source.source) {
            Ok(tokens) => tokens,
            Err(mut errors) => {
                diagnostics.append(&mut errors);
                continue;
            }
        };
        let tokens = match reader::expand(&source.path, &tokens) {
            Ok(tokens) => tokens,
            Err(mut errors) => {
                diagnostics.append(&mut errors);
                continue;
            }
        };
        let forms = match parser::parse(&source.path, &tokens) {
            Ok(forms) => forms,
            Err(mut errors) => {
                diagnostics.append(&mut errors);
                continue;
            }
        };
        let mut program = match ast::build_program(&source.path, &forms) {
            Ok(program) => program,
            Err(mut errors) => {
                diagnostics.append(&mut errors);
                continue;
            }
        };
        // Before anything types it: a declaration for another target is not
        // part of this program (`D-136`).
        ast::select_for_target(&mut program, &options.target);
        units.push(ModuleUnit {
            path: source.path.clone(),
            module: full_module,
            namespace: source.namespace.clone(),
            source_len: source.source.len(),
            base: 0,
            program,
            declarations: HashMap::new(),
            imports: HashMap::new(),
        });
    }

    if !seen_modules.contains_key(&input.entry_module) {
        diagnostics.push(Diagnostic::error(
            codes::MODULE,
            &input.name,
            Span::default(),
            format!("entry module `{}` does not exist", input.entry_module),
        ));
    }
    if !diagnostics.is_empty() {
        return PackageAnalysis {
            diagnostics,
            program: None,
            modules: Vec::new(),
        };
    }

    collect_declarations(&mut units, &input.entry_module, &mut diagnostics);
    let module_indices = units
        .iter()
        .enumerate()
        .map(|(index, unit)| (unit.module.clone(), index))
        .collect::<HashMap<_, _>>();
    let edges = dependency_edges(&units, &module_indices);
    reject_cycles(&units, &edges, &mut diagnostics);
    let exports = resolve_exports(&units, &module_indices, &mut diagnostics);
    resolve_imports(&mut units, &module_indices, &exports, &mut diagnostics);
    if !diagnostics.is_empty() {
        return PackageAnalysis {
            diagnostics,
            program: None,
            modules: summaries(&units, &exports),
        };
    }

    for unit in &mut units {
        let resolver = Resolver {
            unit,
            module_indices: &module_indices,
            exports: &exports,
        };
        let mut program = unit.program.clone();
        resolver.rewrite_program(&mut program, &mut diagnostics);
        unit.program = program;
    }
    if !diagnostics.is_empty() {
        return PackageAnalysis {
            diagnostics,
            program: None,
            modules: summaries(&units, &exports),
        };
    }

    // Every unit that reached here corresponds to an entry of `input.files`, in
    // order: the loop above returns early if it had to skip any.
    for (unit, base) in units.iter_mut().zip(source_bases(&input.files)) {
        unit.base = base;
        shift_program(&mut unit.program, unit.base);
    }
    let mut merged = Program {
        exports: Vec::new(),
        takes: Vec::new(),
        functions: Vec::new(),
        externs: Vec::new(),
        tests: Vec::new(),
        structs: Vec::new(),
        enums: Vec::new(),
        consts: Vec::new(),
        omitted: Vec::new(),
    };
    for (unit, source) in units.iter_mut().zip(&input.files) {
        merged.omitted.append(&mut unit.program.omitted);
        merged.functions.append(&mut unit.program.functions);
        merged.externs.append(&mut unit.program.externs);
        // A dependency's tests belong to the dependency. Collecting them here
        // would run another package's suite from this one's binary, and — since
        // codegen only emits the test bodies owned by the module it is
        // compiling — would leave the harness calling functions no object
        // defines.
        if source.namespace.is_none() {
            merged.tests.append(&mut unit.program.tests);
        }
        merged.structs.append(&mut unit.program.structs);
        merged.enums.append(&mut unit.program.enums);
        merged.consts.append(&mut unit.program.consts);
    }

    let language_items = resolved_language_items(
        &options.language_items,
        &module_indices,
        &exports,
        &mut diagnostics,
        &input.name,
    );
    if !diagnostics.is_empty() {
        return PackageAnalysis {
            diagnostics,
            program: None,
            modules: summaries(&units, &exports),
        };
    }
    let mut warnings = Vec::new();
    match sema::analyze_with_options(
        &input.name,
        &merged,
        &language_items,
        options.validate_entry_point,
        &mut warnings,
    ) {
        Ok(program) => PackageAnalysis {
            diagnostics: reported_warnings(warnings, &units, options),
            program: Some(program),
            modules: summaries(&units, &exports),
        },
        Err(errors) => PackageAnalysis {
            diagnostics: errors
                .into_iter()
                .map(|diagnostic| remap_diagnostic(diagnostic, &units))
                .collect(),
            program: None,
            modules: summaries(&units, &exports),
        },
    }
}

fn resolved_language_items(
    items: &crate::LanguageItems,
    modules: &HashMap<String, usize>,
    exports: &[HashMap<String, String>],
    diagnostics: &mut Vec<Diagnostic>,
    package: &str,
) -> crate::LanguageItems {
    let mut resolve = |name: &str, value: &Option<String>| {
        value.as_ref().map(|path| {
            if module_for_path(path, modules).is_some() {
                resolve_public_path(path, modules, exports).unwrap_or_else(|| {
                    diagnostics.push(Diagnostic::error(
                        codes::STANDARD_LIBRARY,
                        package,
                        Span::default(),
                        format!("language item `{name}` points to private or unknown `{path}`"),
                    ));
                    path.clone()
                })
            } else {
                path.clone()
            }
        })
    };
    crate::LanguageItems {
        option: resolve("option", &items.option),
        result: resolve("result", &items.result),
        result_ok: resolve("result-ok", &items.result_ok),
        result_err: resolve("result-err", &items.result_err),
    }
}

pub fn compile_package_to_hir(
    input: &PackageInput,
    options: &CompileOptions,
) -> CompileResult<TypedProgram> {
    let analysis = analyze_package(input, options);
    match analysis.program {
        Some(program) => Ok(program),
        None => Err(analysis.diagnostics),
    }
}

fn collect_declarations(
    units: &mut [ModuleUnit],
    entry_module: &str,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for unit in units {
        let module = unit.module.clone();
        for (name, kind, span) in unit
            .program
            .functions
            .iter()
            .map(|item| (&item.name, DeclarationKind::Function, item.span))
            .chain(
                unit.program
                    .structs
                    .iter()
                    .map(|item| (&item.name, DeclarationKind::Struct, item.span)),
            )
            .chain(
                unit.program
                    .enums
                    .iter()
                    .map(|item| (&item.name, DeclarationKind::Enum, item.span)),
            )
            .chain(
                unit.program
                    .externs
                    .iter()
                    .map(|item| (&item.name, DeclarationKind::Extern, item.span)),
            )
            .chain(
                unit.program
                    .consts
                    .iter()
                    .map(|item| (&item.name, DeclarationKind::Const, item.span)),
            )
        {
            // A builtin name is never rewritten to a canonical one, so a call
            // to it reaches the builtin and the declaration below it is
            // unreachable. For an `extern` that is worth refusing: the whole
            // point of the declaration is the call.
            if kind == DeclarationKind::Extern && is_builtin(name) {
                diagnostics.push(Diagnostic::error(
                    codes::MODULE,
                    &unit.path,
                    span,
                    format!("`{name}` is a builtin, and an `extern` cannot take its name"),
                ));
            }
            let canonical =
                if module == entry_module && name == "main" && kind == DeclarationKind::Function {
                    "main".to_owned()
                } else {
                    format!("{module}:{name}")
                };
            if let Some(previous) = unit.declarations.insert(
                name.clone(),
                Decl {
                    canonical,
                    kind,
                    span,
                },
            ) {
                diagnostics.push(
                    Diagnostic::error(
                        codes::MODULE,
                        &unit.path,
                        span,
                        format!("top-level name `{name}` is defined more than once"),
                    )
                    .with_label(previous.span, "previous definition is here"),
                );
            }
        }
    }
}

fn dependency_edges(
    units: &[ModuleUnit],
    module_indices: &HashMap<String, usize>,
) -> Vec<Vec<usize>> {
    let mut edges = vec![Vec::new(); units.len()];
    for (index, unit) in units.iter().enumerate() {
        let mut targets = HashSet::new();
        for take in &unit.program.takes {
            let module = module_path_for_unit(unit, &take.module, module_indices);
            if let Some(target) = module_indices.get(&module) {
                targets.insert(*target);
            }
        }
        for export in &unit.program.exports {
            for item in &export.items {
                let path = path_for_unit(unit, &item.path, module_indices);
                if let Some(target) = module_for_path(&path, module_indices) {
                    if target != index {
                        targets.insert(target);
                    }
                }
            }
        }
        let mut names = Vec::new();
        collect_qualified_names(&unit.program, &mut names);
        for name in names {
            let name = path_for_unit(unit, name, module_indices);
            if let Some(target) = module_for_path(&name, module_indices) {
                if target != index {
                    targets.insert(target);
                }
            }
        }
        edges[index] = targets.into_iter().collect();
        edges[index].sort_unstable();
    }
    edges
}

fn reject_cycles(units: &[ModuleUnit], edges: &[Vec<usize>], diagnostics: &mut Vec<Diagnostic>) {
    fn visit(
        node: usize,
        edges: &[Vec<usize>],
        state: &mut [u8],
        stack: &mut Vec<usize>,
    ) -> Option<Vec<usize>> {
        state[node] = 1;
        stack.push(node);
        for &next in &edges[node] {
            if state[next] == 0 {
                if let Some(cycle) = visit(next, edges, state, stack) {
                    return Some(cycle);
                }
            } else if state[next] == 1 {
                let start = stack.iter().position(|item| *item == next).unwrap_or(0);
                let mut cycle = stack[start..].to_vec();
                cycle.push(next);
                return Some(cycle);
            }
        }
        stack.pop();
        state[node] = 2;
        None
    }

    let mut state = vec![0; units.len()];
    for node in 0..units.len() {
        if state[node] != 0 {
            continue;
        }
        if let Some(cycle) = visit(node, edges, &mut state, &mut Vec::new()) {
            let path = cycle
                .iter()
                .map(|index| units[*index].module.as_str())
                .collect::<Vec<_>>()
                .join(" -> ");
            diagnostics.push(
                Diagnostic::error(
                    codes::MODULE,
                    &units[node].path,
                    Span::default(),
                    format!("module dependency cycle: {path}"),
                )
                .with_help("move shared declarations into a third acyclic module"),
            );
            return;
        }
    }
}

fn resolve_exports(
    units: &[ModuleUnit],
    module_indices: &HashMap<String, usize>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<HashMap<String, String>> {
    let mut exports = vec![HashMap::<String, String>::new(); units.len()];
    let mut pending = Vec::<(usize, ImportItem)>::new();
    for (index, unit) in units.iter().enumerate() {
        for declaration in &unit.program.exports {
            for item in &declaration.items {
                if !item.path.contains(':') {
                    if let Some(local) = unit.declarations.get(&item.path) {
                        insert_export(
                            unit,
                            &mut exports[index],
                            &item.alias,
                            &local.canonical,
                            item.span,
                            diagnostics,
                        );
                    } else {
                        diagnostics.push(Diagnostic::error(
                            codes::MODULE,
                            &unit.path,
                            item.span,
                            format!("cannot export unknown local name `{}`", item.path),
                        ));
                    }
                } else if let Some((head, tail)) = item.path.split_once(':') {
                    if let Some(local) = unit.declarations.get(head) {
                        insert_export(
                            unit,
                            &mut exports[index],
                            &item.alias,
                            &format!("{}:{tail}", local.canonical),
                            item.span,
                            diagnostics,
                        );
                    } else {
                        let mut item = item.clone();
                        item.path = path_for_unit(unit, &item.path, module_indices);
                        pending.push((index, item));
                    }
                }
            }
        }
    }

    loop {
        let before = pending.len();
        pending.retain(|(index, item)| {
            let Some(target) = resolve_public_path(&item.path, module_indices, &exports) else {
                return true;
            };
            insert_export(
                &units[*index],
                &mut exports[*index],
                &item.alias,
                &target,
                item.span,
                diagnostics,
            );
            false
        });
        if pending.is_empty() || pending.len() == before {
            break;
        }
    }
    for (index, item) in pending {
        diagnostics.push(
            Diagnostic::error(
                codes::MODULE,
                &units[index].path,
                item.span,
                format!("cannot re-export private or unknown name `{}`", item.path),
            )
            .with_help("export the name from its defining module first"),
        );
    }
    exports
}

fn insert_export(
    unit: &ModuleUnit,
    table: &mut HashMap<String, String>,
    alias: &str,
    canonical: &str,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if table
        .insert(alias.to_owned(), canonical.to_owned())
        .is_some()
    {
        diagnostics.push(Diagnostic::error(
            codes::MODULE,
            &unit.path,
            span,
            format!("exported name `{alias}` is defined more than once"),
        ));
    }
}

fn resolve_imports(
    units: &mut [ModuleUnit],
    module_indices: &HashMap<String, usize>,
    exports: &[HashMap<String, String>],
    diagnostics: &mut Vec<Diagnostic>,
) {
    for unit in units {
        let takes = unit.program.takes.clone();
        for TakeDecl {
            module,
            items,
            span,
        } in takes
        {
            let module = module_path_for_unit(unit, &module, module_indices);
            let Some(_target) = module_indices.get(&module).copied() else {
                diagnostics.push(Diagnostic::error(
                    codes::MODULE,
                    &unit.path,
                    span,
                    format!("unknown module `{module}`"),
                ));
                continue;
            };
            for item in items {
                let path = format!("{module}:{}", item.path);
                let Some(canonical) = resolve_public_path(&path, module_indices, exports) else {
                    diagnostics.push(
                        Diagnostic::error(
                            codes::MODULE,
                            &unit.path,
                            item.span,
                            format!("`{path}` is private or does not exist"),
                        )
                        .with_help("export the name from the source module"),
                    );
                    continue;
                };
                if unit.declarations.contains_key(&item.alias)
                    || unit.imports.insert(item.alias.clone(), canonical).is_some()
                {
                    diagnostics.push(Diagnostic::error(
                        codes::MODULE,
                        &unit.path,
                        item.span,
                        format!("import alias `{}` collides with another name", item.alias),
                    ));
                }
            }
        }
    }
}

struct Resolver<'a> {
    unit: &'a ModuleUnit,
    module_indices: &'a HashMap<String, usize>,
    exports: &'a [HashMap<String, String>],
}

impl Resolver<'_> {
    fn rewrite_program(&self, program: &mut Program, diagnostics: &mut Vec<Diagnostic>) {
        for function in &mut program.functions {
            let original = function.name.clone();
            function.name = self.unit.declarations[&original].canonical.clone();
            for parameter in &mut function.params {
                self.rewrite_type(&mut parameter.ty, parameter.span, diagnostics);
            }
            self.rewrite_type(&mut function.return_type, function.span, diagnostics);
            self.rewrite_expr(&mut function.body, diagnostics);
        }
        for declaration in &mut program.externs {
            let original = declaration.name.clone();
            declaration.name = self.unit.declarations[&original].canonical.clone();
            for parameter in &mut declaration.params {
                self.rewrite_type(&mut parameter.ty, parameter.span, diagnostics);
            }
            self.rewrite_type(&mut declaration.return_type, declaration.span, diagnostics);
        }
        for test in &mut program.tests {
            test.name = format!("{}:{}", self.unit.module, test.name);
            self.rewrite_expr(&mut test.body, diagnostics);
        }
        for structure in &mut program.structs {
            let original = structure.name.clone();
            structure.name = self.unit.declarations[&original].canonical.clone();
            for field in &mut structure.fields {
                self.rewrite_type(&mut field.ty, field.span, diagnostics);
            }
        }
        for enumeration in &mut program.enums {
            let original = enumeration.name.clone();
            enumeration.name = self.unit.declarations[&original].canonical.clone();
            for variant in &mut enumeration.variants {
                for field in &mut variant.fields {
                    self.rewrite_type(&mut field.ty, field.span, diagnostics);
                }
            }
        }
        for constant in &mut program.consts {
            let original = constant.name.clone();
            constant.name = self.unit.declarations[&original].canonical.clone();
            if let Some(ty) = &mut constant.ty {
                self.rewrite_type(ty, constant.span, diagnostics);
            }
        }
        program.exports.clear();
        program.takes.clear();
    }

    fn rewrite_type(&self, ty: &mut Type, span: Span, diagnostics: &mut Vec<Diagnostic>) {
        match ty {
            Type::Named(name) => {
                *name = self.resolve(name, span, diagnostics);
            }
            Type::List(inner) | Type::Slice(inner) | Type::Ref { inner, .. } => {
                self.rewrite_type(inner, span, diagnostics);
            }
            Type::Array { element, .. } => self.rewrite_type(element, span, diagnostics),
            Type::Apply { name, args } => {
                *name = self.resolve(name, span, diagnostics);
                for argument in args {
                    self.rewrite_type(argument, span, diagnostics);
                }
            }
            Type::Fn { params, result } => {
                for param in params {
                    self.rewrite_type(param, span, diagnostics);
                }
                self.rewrite_type(result, span, diagnostics);
            }
            _ => {}
        }
    }

    fn rewrite_expr(&self, expression: &mut Expr, diagnostics: &mut Vec<Diagnostic>) {
        match &mut expression.kind {
            ExprKind::Let { ty, value, .. } => {
                if let Some(ty) = ty {
                    self.rewrite_type(ty, expression.span, diagnostics);
                }
                self.rewrite_expr(value, diagnostics);
            }
            ExprKind::Set { value, .. } => self.rewrite_expr(value, diagnostics),
            ExprKind::Defer(body) => self.rewrite_expr(body, diagnostics),
            ExprKind::Do(items) | ExprKind::Unsafe(items) => {
                for item in items {
                    self.rewrite_expr(item, diagnostics);
                }
            }
            ExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                self.rewrite_expr(condition, diagnostics);
                self.rewrite_expr(then_expr, diagnostics);
                self.rewrite_expr(else_expr, diagnostics);
            }
            ExprKind::Loop { body } => self.rewrite_expr(body, diagnostics),
            ExprKind::While { condition, body } => {
                self.rewrite_expr(condition, diagnostics);
                self.rewrite_expr(body, diagnostics);
            }
            ExprKind::Match { value, arms } => {
                self.rewrite_expr(value, diagnostics);
                for MatchArm {
                    pattern,
                    guard,
                    body,
                    ..
                } in arms
                {
                    self.rewrite_pattern(pattern, diagnostics);
                    if let Some(guard) = guard {
                        self.rewrite_expr(guard, diagnostics);
                    }
                    self.rewrite_expr(body, diagnostics);
                }
            }
            ExprKind::Borrow { value, .. } | ExprKind::Try(value) => {
                self.rewrite_expr(value, diagnostics);
            }
            // `as` carries a type, and the table `D-090` allows holds only
            // scalars, so nothing needs resolving today. It is resolved anyway,
            // because the day the table gains a named row is not the day anyone
            // will remember this arm.
            ExprKind::Convert { target, value } => {
                self.rewrite_type(target, expression.span, diagnostics);
                self.rewrite_expr(value, diagnostics);
            }
            ExprKind::Call { callee, args } => {
                if !is_builtin(callee) {
                    *callee = self.resolve(callee, expression.span, diagnostics);
                }
                for argument in args {
                    self.rewrite_expr(argument, diagnostics);
                }
            }
            ExprKind::Logical { operands, .. } => {
                for operand in operands {
                    self.rewrite_expr(operand, diagnostics);
                }
            }
            // Each operand is a `Var`, so the arm that resolves one resolves a
            // module-qualified composition operand too (`D-139`).
            ExprKind::Compose { operands, .. } => {
                for operand in operands {
                    self.rewrite_expr(operand, diagnostics);
                }
            }
            // A capture is a local by construction, so there is nothing to
            // resolve about the names; the types are another matter, since a
            // parameter or a result may name something from another module.
            ExprKind::Lambda {
                params,
                result,
                body,
                ..
            } => {
                for param in params.iter_mut() {
                    self.rewrite_type(&mut param.ty, param.span, diagnostics);
                }
                self.rewrite_type(result, expression.span, diagnostics);
                self.rewrite_expr(body, diagnostics);
            }
            // A bare name is usually a local, and this pass has no scopes to
            // tell one from a `fn` used as a value (`D-092`). So it records
            // what the name *would* mean as a top-level item and lets sema,
            // which does have scopes, decide — the name itself is untouched.
            //
            // Resolution is silent, because the two errors it can raise are
            // about qualified names and a local is not one: "private or does
            // not exist" would fire on an ordinary local that shares a name
            // with a module. The `::` separator is the exception — no local can
            // be spelled with one, so that error is kept rather than swallowed,
            // and `D-035` stays refused in value position as it is everywhere
            // else.
            ExprKind::Var { name, resolved } => {
                if !is_builtin(name) {
                    let mut quiet = Vec::new();
                    let sink = if name.contains("::") {
                        &mut *diagnostics
                    } else {
                        &mut quiet
                    };
                    *resolved = Some(self.resolve(name, expression.span, sink));
                }
            }
            ExprKind::Unit
            | ExprKind::Bool(_)
            | ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::String(_)
            | ExprKind::Continue => {}
            ExprKind::Break(value) => {
                if let Some(value) = value {
                    self.rewrite_expr(value, diagnostics);
                }
            }
        }
    }

    fn rewrite_pattern(&self, pattern: &mut Pattern, diagnostics: &mut Vec<Diagnostic>) {
        match &mut pattern.kind {
            crate::ast::PatternKind::Enum { path, fields } => {
                *path = self.resolve(path, pattern.span, diagnostics);
                for field in fields {
                    self.rewrite_pattern(field, diagnostics);
                }
            }
            crate::ast::PatternKind::Struct { path, fields } => {
                *path = self.resolve(path, pattern.span, diagnostics);
                for field in fields {
                    self.rewrite_pattern(&mut field.pattern, diagnostics);
                }
            }
            _ => {}
        }
    }

    fn resolve(&self, name: &str, span: Span, diagnostics: &mut Vec<Diagnostic>) -> String {
        if name.contains("::") {
            diagnostics.push(
                Diagnostic::error(
                    codes::MODULE,
                    &self.unit.path,
                    span,
                    "`::` is no longer a valid qualified-name separator",
                )
                .with_suggestion(
                    span,
                    name.replace("::", ":"),
                    "replace `::` with `:`",
                    crate::diagnostic::Applicability::MachineApplicable,
                ),
            );
            return name.replace("::", ":");
        }
        if let Some(local) = self.unit.declarations.get(name) {
            return local.canonical.clone();
        }
        if let Some(imported) = self.unit.imports.get(name) {
            return imported.clone();
        }
        if let Some((head, tail)) = name.split_once(':') {
            if let Some(local) = self.unit.declarations.get(head) {
                return format!("{}:{tail}", local.canonical);
            }
            if let Some(imported) = self.unit.imports.get(head) {
                return format!("{imported}:{tail}");
            }
        }
        if let Some(public) = resolve_public_path(name, self.module_indices, self.exports) {
            return public;
        }
        let local_path = path_for_unit(self.unit, name, self.module_indices);
        if local_path != name {
            if let Some(public) =
                resolve_public_path(&local_path, self.module_indices, self.exports)
            {
                return public;
            }
        }
        if module_for_path(name, self.module_indices).is_some() {
            diagnostics.push(
                Diagnostic::error(
                    codes::MODULE,
                    &self.unit.path,
                    span,
                    format!("qualified name `{name}` is private or does not exist"),
                )
                .with_help("export the name or introduce a valid alias with `take`"),
            );
        }
        name.to_owned()
    }
}

fn resolve_public_path(
    path: &str,
    modules: &HashMap<String, usize>,
    exports: &[HashMap<String, String>],
) -> Option<String> {
    let (module, index) = longest_module_prefix(path, modules)?;
    let remainder = path.strip_prefix(module)?.strip_prefix(':')?;
    let (name, suffix) = remainder
        .split_once(':')
        .map_or((remainder, None), |(name, suffix)| (name, Some(suffix)));
    let base = exports[index].get(name)?.clone();
    Some(match suffix {
        Some(suffix) => format!("{base}:{suffix}"),
        None => base,
    })
}

fn module_for_path(path: &str, modules: &HashMap<String, usize>) -> Option<usize> {
    longest_module_prefix(path, modules).map(|(_, index)| index)
}

fn module_path_for_unit(unit: &ModuleUnit, path: &str, modules: &HashMap<String, usize>) -> String {
    if modules.contains_key(path) {
        return path.to_owned();
    }
    if let Some(namespace) = &unit.namespace {
        let local = format!("{namespace}:{path}");
        if modules.contains_key(&local) {
            return local;
        }
    }
    path.to_owned()
}

fn path_for_unit(unit: &ModuleUnit, path: &str, modules: &HashMap<String, usize>) -> String {
    if module_for_path(path, modules).is_some() {
        return path.to_owned();
    }
    if let Some(namespace) = &unit.namespace {
        let local = format!("{namespace}:{path}");
        if module_for_path(&local, modules).is_some() {
            return local;
        }
    }
    path.to_owned()
}

fn longest_module_prefix<'a>(
    path: &str,
    modules: &'a HashMap<String, usize>,
) -> Option<(&'a str, usize)> {
    modules
        .iter()
        .filter(|(module, _)| {
            path.len() > module.len()
                && path.starts_with(module.as_str())
                && path.as_bytes().get(module.len()) == Some(&b':')
        })
        .max_by_key(|(module, _)| module.len())
        .map(|(module, index)| (module.as_str(), *index))
}

fn collect_qualified_names<'a>(program: &'a Program, output: &mut Vec<&'a str>) {
    fn ty<'a>(type_: &'a Type, output: &mut Vec<&'a str>) {
        match type_ {
            Type::Named(name) if name.contains(':') => output.push(name),
            Type::List(inner) | Type::Slice(inner) | Type::Ref { inner, .. } => ty(inner, output),
            Type::Array { element, .. } => ty(element, output),
            Type::Apply { name, args } => {
                if name.contains(':') {
                    output.push(name);
                }
                args.iter().for_each(|argument| ty(argument, output));
            }
            Type::Fn { params, result } => {
                params.iter().for_each(|param| ty(param, output));
                ty(result, output);
            }
            _ => {}
        }
    }
    fn expr<'a>(expression: &'a Expr, output: &mut Vec<&'a str>) {
        fn pattern<'a>(item: &'a Pattern, output: &mut Vec<&'a str>) {
            match &item.kind {
                crate::ast::PatternKind::Enum { path, fields } => {
                    output.push(path);
                    fields.iter().for_each(|field| pattern(field, output));
                }
                crate::ast::PatternKind::Struct { path, fields } => {
                    output.push(path);
                    fields
                        .iter()
                        .for_each(|field| pattern(&field.pattern, output));
                }
                _ => {}
            }
        }
        match &expression.kind {
            ExprKind::Let {
                ty: written, value, ..
            } => {
                if let Some(written) = written {
                    ty(written, output);
                }
                expr(value, output);
            }
            ExprKind::Set { value, .. } => expr(value, output),
            ExprKind::Do(items) | ExprKind::Unsafe(items) => {
                items.iter().for_each(|item| expr(item, output))
            }
            ExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                expr(condition, output);
                expr(then_expr, output);
                expr(else_expr, output);
            }
            ExprKind::Loop { body } => expr(body, output),
            ExprKind::While { condition, body } => {
                expr(condition, output);
                expr(body, output);
            }
            ExprKind::Match { value, arms } => {
                expr(value, output);
                for arm in arms {
                    pattern(&arm.pattern, output);
                    if let Some(guard) = &arm.guard {
                        expr(guard, output);
                    }
                    expr(&arm.body, output);
                }
            }
            ExprKind::Break(Some(value)) => expr(value, output),
            ExprKind::Borrow { value, .. }
            | ExprKind::Try(value)
            | ExprKind::Convert { value, .. } => expr(value, output),
            ExprKind::Call { callee, args } => {
                if callee.contains(':') {
                    output.push(callee);
                }
                args.iter().for_each(|argument| expr(argument, output));
            }
            ExprKind::Logical { operands, .. } => {
                operands.iter().for_each(|operand| expr(operand, output));
            }
            ExprKind::Compose { operands, .. } => {
                operands.iter().for_each(|operand| expr(operand, output));
            }
            ExprKind::Lambda {
                params,
                result,
                body,
                ..
            } => {
                params.iter().for_each(|param| ty(&param.ty, output));
                ty(result, output);
                expr(body, output);
            }
            _ => {}
        }
    }
    for function in &program.functions {
        function
            .params
            .iter()
            .for_each(|param| ty(&param.ty, output));
        ty(&function.return_type, output);
        expr(&function.body, output);
    }
    for structure in &program.structs {
        structure
            .fields
            .iter()
            .for_each(|field| ty(&field.ty, output));
    }
    for enumeration in &program.enums {
        for variant in &enumeration.variants {
            variant
                .fields
                .iter()
                .for_each(|field| ty(&field.ty, output));
        }
    }
    for declaration in &program.externs {
        declaration
            .params
            .iter()
            .for_each(|parameter| ty(&parameter.ty, output));
        ty(&declaration.return_type, output);
    }
    for test in &program.tests {
        expr(&test.body, output);
    }
    for constant in &program.consts {
        if let Some(written) = &constant.ty {
            ty(written, output);
        }
    }
}

fn summaries(units: &[ModuleUnit], exports: &[HashMap<String, String>]) -> Vec<ModuleSummary> {
    units
        .iter()
        .enumerate()
        .map(|(index, unit)| {
            let mut names = exports[index].keys().cloned().collect::<Vec<_>>();
            names.sort();
            ModuleSummary {
                path: unit.path.clone(),
                module: unit.module.clone(),
                exports: names,
                export_bindings: sorted_bindings(&exports[index]),
                imports: sorted_bindings(&unit.imports),
                declarations: {
                    let mut declarations = unit
                        .declarations
                        .iter()
                        .map(|(name, declaration)| ModuleDeclaration {
                            name: name.clone(),
                            canonical: declaration.canonical.clone(),
                            kind: declaration.kind,
                            span: declaration.span,
                        })
                        .collect::<Vec<_>>();
                    declarations.sort_by(|left, right| left.name.cmp(&right.name));
                    declarations
                },
            }
        })
        .collect()
}

fn sorted_bindings(bindings: &HashMap<String, String>) -> Vec<ModuleBinding> {
    let mut bindings = bindings
        .iter()
        .map(|(name, canonical)| ModuleBinding {
            name: name.clone(),
            canonical: canonical.clone(),
        })
        .collect::<Vec<_>>();
    bindings.sort_by(|left, right| left.name.cmp(&right.name));
    bindings
}

fn valid_module_name(name: &str) -> bool {
    !name.is_empty()
        && name.split(':').all(|segment| {
            !segment.is_empty()
                && segment.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
                })
        })
}

/// Whether a name in head position belongs to the language rather than to a
/// module.
///
/// The operators come from `sema`'s table rather than being listed again here:
/// there were two copies of the seven, and two copies of eighteen would have
/// drifted on the first patch that added a nineteenth.
fn is_builtin(name: &str) -> bool {
    crate::sema::is_operator(name)
        || matches!(
            name,
            "clone"
                | "list"
                | "array"
                | "slice"
                | "len"
                | "push"
                | "get"
                | "get-ref"
                | "pop"
                | "remove"
                | "replace"
                | "."
                | "volatile-read"
                | "volatile-write"
                | "ptr-offset"
        )
}

fn shift_span(span: &mut Span, base: usize) {
    span.start += base;
    span.end += base;
}

fn shift_program(program: &mut Program, base: usize) {
    fn shift_type(_ty: &mut Type, _base: usize) {}
    fn shift_pattern(pattern: &mut Pattern, base: usize) {
        shift_span(&mut pattern.span, base);
        match &mut pattern.kind {
            crate::ast::PatternKind::Enum { fields, .. } => {
                fields
                    .iter_mut()
                    .for_each(|field| shift_pattern(field, base));
            }
            crate::ast::PatternKind::Struct { fields, .. } => {
                // The keyword's span moves with the module the same way every
                // other one does; a span that stayed behind would point a
                // warning at whatever happened to sit at that offset.
                fields.iter_mut().for_each(|field| {
                    shift_span(&mut field.span, base);
                    shift_pattern(&mut field.pattern, base);
                });
            }
            _ => {}
        }
    }
    fn shift_expr(expression: &mut Expr, base: usize) {
        shift_span(&mut expression.span, base);
        match &mut expression.kind {
            ExprKind::Let { ty, value, .. } => {
                if let Some(ty) = ty {
                    shift_type(ty, base);
                }
                shift_expr(value, base);
            }
            ExprKind::Set { value, .. } => shift_expr(value, base),
            ExprKind::Do(items) | ExprKind::Unsafe(items) => {
                items.iter_mut().for_each(|item| shift_expr(item, base))
            }
            ExprKind::If {
                condition,
                then_expr,
                else_expr,
            } => {
                shift_expr(condition, base);
                shift_expr(then_expr, base);
                shift_expr(else_expr, base);
            }
            ExprKind::Loop { body } => shift_expr(body, base),
            ExprKind::While { condition, body } => {
                shift_expr(condition, base);
                shift_expr(body, base);
            }
            ExprKind::Match { value, arms } => {
                shift_expr(value, base);
                for arm in arms {
                    shift_span(&mut arm.span, base);
                    shift_pattern(&mut arm.pattern, base);
                    if let Some(guard) = &mut arm.guard {
                        shift_expr(guard, base);
                    }
                    shift_expr(&mut arm.body, base);
                }
            }
            ExprKind::Break(Some(value)) => shift_expr(value, base),
            ExprKind::Borrow { value, .. }
            | ExprKind::Try(value)
            | ExprKind::Convert { value, .. } => shift_expr(value, base),
            ExprKind::Call { args, .. } => {
                args.iter_mut()
                    .for_each(|argument| shift_expr(argument, base));
            }
            ExprKind::Logical { operands, .. } => {
                operands
                    .iter_mut()
                    .for_each(|operand| shift_expr(operand, base));
            }
            ExprKind::Lambda {
                captures,
                params,
                result,
                body,
            } => {
                for capture in captures.iter_mut() {
                    shift_span(&mut capture.span, base);
                }
                for param in params.iter_mut() {
                    shift_span(&mut param.span, base);
                    shift_type(&mut param.ty, base);
                }
                shift_type(result, base);
                shift_expr(body, base);
            }
            _ => {}
        }
    }
    for function in &mut program.functions {
        shift_span(&mut function.span, base);
        for parameter in &mut function.params {
            shift_span(&mut parameter.span, base);
            shift_type(&mut parameter.ty, base);
        }
        shift_type(&mut function.return_type, base);
        shift_expr(&mut function.body, base);
    }
    for declaration in &mut program.externs {
        shift_span(&mut declaration.span, base);
        shift_span(&mut declaration.symbol_span, base);
        for parameter in &mut declaration.params {
            shift_span(&mut parameter.span, base);
        }
    }
    for test in &mut program.tests {
        shift_span(&mut test.span, base);
        shift_expr(&mut test.body, base);
    }
    for structure in &mut program.structs {
        shift_span(&mut structure.span, base);
        for field in &mut structure.fields {
            shift_span(&mut field.span, base);
        }
    }
    for enumeration in &mut program.enums {
        shift_span(&mut enumeration.span, base);
        for variant in &mut enumeration.variants {
            shift_span(&mut variant.span, base);
            for field in &mut variant.fields {
                shift_span(&mut field.span, base);
            }
        }
    }
    for constant in &mut program.consts {
        shift_span(&mut constant.span, base);
        if let Some(ty) = &mut constant.ty {
            shift_type(ty, base);
        }
        shift_expr(&mut constant.value, base);
    }
}

/// Which of a compilation's warnings this compilation is the one to report
/// (`D-122`).
///
/// Two rules, and both of them exist because `sema` is handed the whole
/// package however little of it is being built. A warning about a dependency's
/// own source is the dependency's business, the same reading that already
/// keeps a dependency's tests out of this package's harness. And when a
/// codegen module is named — which is how `slopium` builds, one `slopic` per
/// object — only that module's warnings belong to this run, or every warning
/// in the package would be printed once per object.
fn reported_warnings(
    warnings: Vec<Diagnostic>,
    units: &[ModuleUnit],
    options: &CompileOptions,
) -> Vec<Diagnostic> {
    warnings
        .into_iter()
        .filter_map(|warning| {
            let unit = owning_unit(&warning, units)?;
            if unit.namespace.is_some() {
                return None;
            }
            if let Some(selected) = &options.codegen_module {
                if &unit.module != selected {
                    return None;
                }
            }
            Some(remap_diagnostic(warning, units))
        })
        .collect()
}

/// The module whose source a merged span came from.
fn owning_unit<'a>(diagnostic: &Diagnostic, units: &'a [ModuleUnit]) -> Option<&'a ModuleUnit> {
    units.iter().find(|unit| {
        diagnostic.span.start >= unit.base && diagnostic.span.start <= unit.base + unit.source_len
    })
}

fn remap_diagnostic(mut diagnostic: Diagnostic, units: &[ModuleUnit]) -> Diagnostic {
    let unit = owning_unit(&diagnostic, units).unwrap_or(&units[0]);
    diagnostic.file = unit.path.clone();
    diagnostic.span.start = diagnostic.span.start.saturating_sub(unit.base);
    diagnostic.span.end = diagnostic.span.end.saturating_sub(unit.base);
    for label in &mut diagnostic.labels {
        label.span.start = label.span.start.saturating_sub(unit.base);
        label.span.end = label.span.end.saturating_sub(unit.base);
    }
    for suggestion in &mut diagnostic.suggestions {
        suggestion.span.start = suggestion.span.start.saturating_sub(unit.base);
        suggestion.span.end = suggestion.span.end.saturating_sub(unit.base);
    }
    diagnostic
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(module: &str, text: &str) -> PackageSource {
        PackageSource {
            path: format!("src/{module}.slp"),
            namespace: None,
            module: module.to_owned(),
            source: text.to_owned(),
        }
    }

    #[test]
    fn resolves_exports_takes_and_qualified_calls() {
        let input = PackageInput {
            name: "demo".into(),
            entry_module: "main".into(),
            files: vec![
                source(
                    "geometry",
                    "(export distance)\n(fn distance ((n i64)) -> i64 (+ n 1))",
                ),
                source(
                    "main",
                    "(take geometry (distance :as length))\n(fn main () -> i64 (+ (length 1) (geometry:distance 2)))",
                ),
            ],
        };
        let analysis = analyze_package(&input, &CompileOptions::default());
        assert!(
            analysis.diagnostics.is_empty(),
            "{:?}",
            analysis.diagnostics
        );
        let program = analysis.program.unwrap();
        assert!(program
            .functions
            .iter()
            .any(|function| function.name == "geometry:distance"));
    }

    #[test]
    fn rejects_private_access_and_cycles() {
        let private = PackageInput {
            name: "demo".into(),
            entry_module: "main".into(),
            files: vec![
                source("geometry", "(fn hidden () -> i64 1)"),
                source("main", "(fn main () -> i64 (geometry:hidden))"),
            ],
        };
        assert!(analyze_package(&private, &CompileOptions::default())
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("private")));

        let cycle = PackageInput {
            name: "demo".into(),
            entry_module: "main".into(),
            files: vec![
                source("a", "(export a)\n(take b b)\n(fn a () -> i64 (b))"),
                source("b", "(export b)\n(take a a)\n(fn b () -> i64 (a))"),
                source("main", "(take a a)\n(fn main () -> i64 (a))"),
            ],
        };
        assert!(analyze_package(&cycle, &CompileOptions::default())
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("cycle")));
    }

    /// Every statement of a lowered function must resolve to the file whose
    /// module its symbol names. That is what a debug line table rests on: a
    /// base rule off by a single byte would attribute the opening declaration
    /// of one module to the end of the one before it.
    ///
    /// Each module therefore opens with its function at byte 0, which is the
    /// only offset sensitive to a one-byte drift — spans further in have slack
    /// and would keep resolving correctly.
    #[test]
    fn every_lowered_span_resolves_to_the_module_it_was_written_in() {
        let input = PackageInput {
            name: "demo".into(),
            entry_module: "main".into(),
            files: vec![
                source("alpha", "(fn one () -> i64 1)\n(export one)\n"),
                source(
                    "beta",
                    "(fn two () -> i64\n  (+ (one) (one)))\n(take alpha one)\n(export two)\n",
                ),
                source(
                    "main",
                    "(fn main () -> i32\n  (let sum (two))\n  0)\n(take beta two)\n",
                ),
            ],
        };
        let map = source_map(&input);
        let paths: Vec<&str> = map.paths().collect();
        assert_eq!(paths, ["src/alpha.slp", "src/beta.slp", "src/main.slp"]);

        let module = crate::compile_package_to_mir(&input, &CompileOptions::default()).unwrap();
        let mut checked = 0;
        let mut at_a_boundary = HashSet::new();
        let bases = source_bases(&input.files);
        for function in &module.functions {
            // `main` carries no module prefix; every other symbol does.
            let expected = match function.name.rsplit_once(':') {
                Some((module, _)) => format!("src/{module}.slp"),
                None => "src/main.slp".to_owned(),
            };
            for block in &function.blocks {
                for span in block
                    .statements
                    .iter()
                    .map(|statement| statement.span)
                    .chain([block.terminator_span, function.span])
                {
                    if span.line == 0 {
                        continue;
                    }
                    let index = map.index_of(span).expect("the map has three files");
                    assert_eq!(
                        paths[index], expected,
                        "`{}` has a span at offset {} attributed to the wrong module",
                        function.name, span.start
                    );
                    checked += 1;
                    if bases.contains(&span.start) {
                        at_a_boundary.insert(span.start);
                    }
                }
            }
        }
        assert!(checked > 0, "no spans were checked");
        assert_eq!(
            at_a_boundary.len(),
            3,
            "each module should contribute one span at its own first byte, \
             which is what makes this test sensitive to the base rule"
        );
    }
}
