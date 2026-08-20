pub mod aarch64;
pub mod aarch64_inst;
pub mod analysis;
pub mod asm;
pub mod ast;
pub mod cfg;
pub mod codegen;
pub mod diagnostic;
pub mod elf;
pub mod lexer;
pub mod lowering;
pub mod mir;
pub mod mir_print;
pub mod opt;
pub mod package;
pub mod parser;
pub mod regalloc;
pub mod sema;
pub mod syntax;
pub mod verify;
pub mod x86_64_inst;

use crate::codegen::{CodegenOptions, Environment, DEFAULT_TARGET, TARGET_TRIPLES};
use crate::diagnostic::{codes, CompileResult, Diagnostic, SourceMap};
use crate::mir::MirModule;
use crate::sema::TypedProgram;
use serde::Serialize;
pub use slopium_std::{language_items_of, CORE_PACKAGE, STD_PACKAGE, TOOLCHAIN_PACKAGES};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const COMPILER_PROTOCOL: u32 = 8;
pub const STANDARD_LIBRARY_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The half of the runtime a freestanding program can have: strings, lists,
/// slices, and the failure paths. It calls four symbols it does not define
/// (`D-066`, `D-080`).
pub const RUNTIME_CORE: (&str, &[u8]) = (
    "slop_rt_core.c",
    include_bytes!("../../../runtime/slop_rt_core.c"),
);

/// The half that defines those four over libc and adds stdio, `argv` and
/// `getenv`.
pub const RUNTIME_HOSTED: (&str, &[u8]) = (
    "slop_rt_hosted.c",
    include_bytes!("../../../runtime/slop_rt_hosted.c"),
);

/// The extra `cc` arguments a freestanding build is compiled with.
///
/// A compiler is free to recognize the byte loops in `slop_rt_core.c` and emit
/// the `memcpy` they were written to avoid, which would be a libc symbol in the
/// half that must not have one; `-ffreestanding -fno-builtin` discourage it.
/// `-fno-stack-protector` is not an optimization question at all — a toolchain
/// that turns the stack protector on by default (nixpkgs does) emits calls to
/// `__stack_chk_fail`, which libc owns, and `core-check.sh` caught it.
///
/// A hosted build uses none of them. Core is linked beside libc there, so a
/// `memcpy` call costs nothing and the hardening is worth keeping.
pub const FREESTANDING_FLAGS: &[&str] = &["-ffreestanding", "-fno-builtin", "-fno-stack-protector"];

/// The environment a build runs in: what the command line asked for, or the
/// target's default when it asked for nothing (`D-081`).
pub fn request_environment(options: &CompileOptions) -> Environment {
    options
        .environment
        .unwrap_or_else(|| environment_for(&options.target))
}

/// The environment a target implies, with no command line to override it.
///
/// `slopium` needs this: it decides which runtime units to materialize and how
/// to link before it has a [`CompileOptions`] to ask about, and a second copy of
/// the lookup is a second place for the answer to differ.
pub fn environment_for(triple: &str) -> Environment {
    codegen::target_spec(triple)
        .map(|spec| spec.environment)
        .unwrap_or_default()
}

/// The runtime units an environment links, in link order.
pub fn runtime_sources(environment: Environment) -> Vec<(&'static str, &'static [u8])> {
    match environment {
        Environment::Hosted => vec![RUNTIME_CORE, RUNTIME_HOSTED],
        Environment::Freestanding => vec![RUNTIME_CORE],
    }
}

#[derive(Clone, Debug)]
pub struct CompileOptions {
    pub target: String,
    pub test_harness: bool,
    pub optimize: bool,
    pub codegen_module: Option<String>,
    pub language_items: LanguageItems,
    pub validate_entry_point: bool,
    /// Emit DWARF line tables, so a debugger can map an address back to the
    /// expression it came from.
    pub debug: bool,
    /// Make a trap abort without a message. A mechanism: the compiler emits
    /// the message-less path when asked; the manager decides when, from the
    /// profile's `panic` setting.
    pub panic_abort: bool,
    /// Remove the symbol table from a linked executable. A mechanism, not a
    /// policy: the compiler strips when asked and does not decide when to ask.
    /// The manager does, from the profile — see `slopium`'s `strip_symbols`.
    pub strip: bool,
    /// What the program can assume is under it. `None` takes the target's
    /// default, which is how `x86_64-unknown-none` selects a freestanding
    /// build; `--freestanding` is the override, and it is still the only way to
    /// reach a freestanding AArch64 object, since no `-none` row claims that
    /// architecture yet (`D-081`).
    pub environment: Option<Environment>,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            target: DEFAULT_TARGET.into(),
            test_harness: false,
            optimize: false,
            codegen_module: None,
            language_items: LanguageItems::default(),
            validate_entry_point: true,
            debug: false,
            panic_abort: false,
            strip: false,
            environment: None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct LanguageItems {
    pub option: Option<String>,
    pub result: Option<String>,
    pub result_ok: Option<String>,
    pub result_err: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmitKind {
    Check,
    Hir,
    Mir,
    /// Same MIR as [`EmitKind::Mir`], rendered for humans instead of machines.
    MirText,
    Assembly,
    Object,
    Executable,
}

#[derive(Clone, Debug)]
pub struct CompileRequest {
    pub input: PathBuf,
    pub source_root: Option<PathBuf>,
    /// Modules under `source_root` this build is not made of (`D-135`).
    ///
    /// The compiler discovers a package's modules by walking, and never reads a
    /// manifest (`D-002`), so target selection reaches it as a list rather than
    /// as a condition: whoever read the manifest decided, and this is the
    /// answer. An entry that names no module is not an error — a package may
    /// name a module for a target whose file another package supplies.
    pub excluded_modules: Vec<String>,
    /// The module name a file under `source_root` has, when the manifest gave
    /// it one rather than letting its path decide (`D-135`).
    ///
    /// This is what makes a module mean a different file per target without
    /// the rest of the program knowing: the file changes, the name does not.
    /// Path derivation (`D-009`) still names every other file.
    pub named_modules: Vec<(String, PathBuf)>,
    pub dependencies: Vec<DependencySource>,
    pub toolchain_dependencies: Vec<String>,
    pub output: Option<PathBuf>,
    pub emit: EmitKind,
    pub options: CompileOptions,
    /// The runtime units to link, replacing the ones bundled with the
    /// compiler. Empty means the bundled ones for this environment; there is
    /// more than one now, so this is a list rather than a path (`D-066`).
    pub runtimes: Vec<PathBuf>,
    pub cc: String,
}

#[derive(Clone, Debug)]
pub struct DependencySource {
    pub namespace: String,
    pub source_root: PathBuf,
    /// Modules of this dependency the build is not made of (`D-135`).
    pub excluded_modules: Vec<String>,
    /// The module name a file of this dependency has, when its manifest gave it
    /// one (`D-135`).
    pub named_modules: Vec<(String, PathBuf)>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CompilerInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub protocol: u32,
    pub targets: &'static [&'static str],
    pub standard_library: &'static str,
}

/// The extra `cc` arguments for a final link, shared by `slopic`'s own link and
/// `slopium`'s.
///
/// `slopium` links the package objects itself rather than through `slopic`, so
/// the two would otherwise each carry their own copy of this list and be free
/// to drift. One function means a program built either way comes out the same.
///
/// `--gc-sections`, paired with the per-function sections the runtime is
/// compiled with, drops every helper nothing reaches — it only ever removes
/// unreferenced code, so it is unconditional. Stripping removes the symbol
/// table a debugger needs, so it is a choice. `-DSLOPIUM_PANIC_ABORT` compiles
/// the runtime's error paths down to a bare exit, matching the message-less
/// trampolines the compiler emits under the same option, so a `panic = "abort"`
/// binary carries no error strings at all.
pub fn cc_flags(environment: Environment, strip: bool, panic_abort: bool) -> Vec<&'static str> {
    let mut flags = cc_compile_flags(environment);
    flags.push("-Wl,--gc-sections");
    if environment == Environment::Freestanding {
        flags.extend_from_slice(FREESTANDING_LINK_FLAGS);
    }
    if strip {
        flags.push("-Wl,--strip-all");
    }
    if panic_abort {
        flags.push("-DSLOPIUM_PANIC_ABORT");
    }
    flags
}

/// What a freestanding link says that a hosted one does not.
///
/// `-nostdlib` drops the C library and the start-up files both; `-nostartfiles`
/// is named beside it anyway, because the pair is what the link *means* and a
/// reader should not have to know that one implies the other. `-static` and
/// `-no-pie` are not tidiness: a toolchain that defaults to `-pie` emits a
/// dynamic object asking for an interpreter, which is the one thing a program
/// with no C library underneath it cannot be given (`core-check.sh` is where
/// that was learned).
///
/// The entry point is not named here. With `-nostartfiles` the linker still
/// looks for `_start`, which is the symbol the program's own stub defines, so
/// `--gc-sections` keeps it as a root and no `-e` is needed.
pub const FREESTANDING_LINK_FLAGS: &[&str] = &["-nostdlib", "-nostartfiles", "-static", "-no-pie"];

/// The half of [`cc_flags`] that survives `cc -c`.
///
/// A package's `c-sources` are compiled to objects of their own and handed to
/// the same link, so they need the per-function sections `--gc-sections` prunes
/// against — but not the `-Wl,` flags, which a compile-only invocation has no
/// linker to pass on to (`D-075`). The environment reaches here because those
/// same `c-sources` hold a freestanding program's entry stub, and compiling it
/// as a hosted translation unit is how a `memcpy` or a stack-protector call
/// appears in the half that must not have one.
pub fn cc_compile_flags(environment: Environment) -> Vec<&'static str> {
    let mut flags = vec!["-ffunction-sections", "-fdata-sections"];
    if environment == Environment::Freestanding {
        flags.extend_from_slice(FREESTANDING_FLAGS);
    }
    flags
}

pub fn compiler_info() -> CompilerInfo {
    CompilerInfo {
        name: "slopic",
        version: env!("CARGO_PKG_VERSION"),
        protocol: COMPILER_PROTOCOL,
        targets: TARGET_TRIPLES,
        standard_library: STANDARD_LIBRARY_VERSION,
    }
}

pub fn compile_to_hir(
    file: &str,
    source: &str,
    options: &CompileOptions,
) -> CompileResult<TypedProgram> {
    let analysis = analysis::analyze_source(file, source, options);
    match analysis.program {
        Some(program) => Ok(program),
        None => Err(analysis.diagnostics),
    }
}

pub fn compile_to_mir(
    file: &str,
    source: &str,
    options: &CompileOptions,
) -> CompileResult<MirModule> {
    lower_and_optimize(file, &compile_to_hir(file, source, options)?, options, None)
}

pub fn compile_package_to_mir(
    input: &package::PackageInput,
    options: &CompileOptions,
) -> CompileResult<MirModule> {
    let program = package::compile_package_to_hir(input, options)?;
    lower_and_optimize(&input.name, &program, options, Some(input))
}

/// Everything between a checked program and the MIR a backend is handed.
///
/// One function rather than two because the file and the package paths differ
/// in exactly one step — the partition a codegen module asks for — and both of
/// them are reached twice: once by the library's own entry points and once by
/// the request path, which carries a compilation's warnings with it (`D-122`).
fn lower_and_optimize(
    name: &str,
    program: &TypedProgram,
    options: &CompileOptions,
    input: Option<&package::PackageInput>,
) -> CompileResult<MirModule> {
    let mut module = mir::lower(program);
    verify::check(name, &module, "lowering")?;
    // `partition_codegen` only flips `emit` flags, so it cannot invalidate any
    // verified invariant and is not worth a second pass — this runs once per
    // owner module per build.
    if let (Some(input), Some(selected)) = (input, &options.codegen_module) {
        partition_codegen(&mut module, input, selected);
    }
    if options.optimize {
        opt::optimize(name, &mut module)?;
    }
    Ok(module)
}

/// What a backend is told about the program it is emitting.
///
/// `line_tables` is false for an object, because the object writer builds none
/// (`D-028`) and a debug build never reaches it; the caller checks that first.
fn codegen_options(
    file: &str,
    options: &CompileOptions,
    input: Option<&package::PackageInput>,
    line_tables: bool,
) -> CodegenOptions {
    // The root module emits the entrypoint, and only in an environment that
    // has something to be entered from (`D-081`).
    let emits_entrypoint = request_environment(options).emits_entrypoint()
        && input.is_none_or(|input| {
            options
                .codegen_module
                .as_ref()
                .is_none_or(|module| module == &input.entry_module)
        });
    CodegenOptions {
        target: options.target.clone(),
        // A lone file's harness does not depend on the entrypoint: there is no
        // module that could be the one to emit it.
        test_harness: options.test_harness && input.is_none_or(|_| emits_entrypoint),
        emit_entrypoint: emits_entrypoint,
        debug: (options.debug && line_tables).then(|| match input {
            Some(input) => package::source_map(input),
            None => SourceMap::single(file),
        }),
        panic_abort: options.panic_abort,
    }
}

pub fn compile_to_assembly(
    file: &str,
    source: &str,
    options: &CompileOptions,
) -> CompileResult<String> {
    let mir = compile_to_mir(file, source, options)?;
    codegen::emit_assembly(file, &mir, &codegen_options(file, options, None, true))
}

pub fn compile_to_object(
    file: &str,
    source: &str,
    options: &CompileOptions,
) -> CompileResult<Vec<u8>> {
    let mir = compile_to_mir(file, source, options)?;
    codegen::emit_object(file, &mir, &codegen_options(file, options, None, false))
}

pub fn compile_package_to_assembly(
    input: &package::PackageInput,
    options: &CompileOptions,
) -> CompileResult<String> {
    let mir = compile_package_to_mir(input, options)?;
    codegen::emit_assembly(
        &input.name,
        &mir,
        &codegen_options(&input.name, options, Some(input), true),
    )
}

pub fn compile_package_to_object(
    input: &package::PackageInput,
    options: &CompileOptions,
) -> CompileResult<Vec<u8>> {
    let mir = compile_package_to_mir(input, options)?;
    codegen::emit_object(
        &input.name,
        &mir,
        &codegen_options(&input.name, options, Some(input), false),
    )
}

fn partition_codegen(module: &mut MirModule, input: &package::PackageInput, selected: &str) {
    let module_names = input
        .files
        .iter()
        .map(|source| {
            source.namespace.as_ref().map_or_else(
                || source.module.clone(),
                |namespace| format!("{namespace}:{}", source.module),
            )
        })
        .collect::<Vec<_>>();
    let owner = |name: &str| {
        if name == "main" {
            return Some(input.entry_module.as_str());
        }
        module_names
            .iter()
            .filter(|module| {
                name.len() > module.len()
                    && name.starts_with(module.as_str())
                    && name.as_bytes().get(module.len()) == Some(&b':')
            })
            .max_by_key(|module| module.len())
            .map(String::as_str)
    };
    // A `lambda` body and its environment are named `<owner>$lambda$n` and
    // `<owner>$closure$n`, so what they belong to is the part before the first
    // `$` — which is the whole reason they are named that way. `main` and a
    // test body have no module in their names at all, and this is what keeps a
    // closure written in one of them from being emitted by nobody.
    let base = |name: &str| name.split('$').next().unwrap_or(name).to_owned();
    let mut emitted = std::collections::HashMap::new();
    for function in &module.functions {
        if !function.name.contains('$') {
            emitted.insert(
                function.name.clone(),
                owner(&function.name) == Some(selected),
            );
        }
    }
    for test in &module.tests {
        emitted.insert(
            test.function.name.clone(),
            owner(&test.name) == Some(selected),
        );
    }
    for function in &mut module.functions {
        function.emit = emitted
            .get(&base(&function.name))
            .copied()
            .unwrap_or_else(|| owner(&function.name) == Some(selected));
    }
    for test in &mut module.tests {
        test.emit = owner(&test.name) == Some(selected);
        test.function.emit = test.emit;
    }
    for structure in &mut module.structs {
        structure.emit = emitted
            .get(&base(&structure.name))
            .copied()
            .unwrap_or_else(|| owner(&structure.name) == Some(selected));
    }
    for enumeration in &mut module.enums {
        enumeration.emit = owner(&enumeration.name) == Some(selected);
    }
}

/// What a compilation produced, and what it has to say about the program it
/// compiled (`D-122`).
///
/// A warning is not an error, so it cannot travel in the `Err` half; and it
/// belongs to the compilation rather than to the program, because which
/// warnings a run reports depends on what it was asked to build.
pub struct Compiled {
    pub output: Option<PathBuf>,
    pub warnings: Vec<Diagnostic>,
}

pub fn compile(request: &CompileRequest) -> CompileResult<Compiled> {
    let mut warnings = Vec::new();
    let output = compile_output(request, &mut warnings)?;
    Ok(Compiled { output, warnings })
}

fn compile_output(
    request: &CompileRequest,
    warnings: &mut Vec<Diagnostic>,
) -> CompileResult<Option<PathBuf>> {
    let file = request.input.display().to_string();
    let source = fs::read_to_string(&request.input).map_err(|error| {
        vec![Diagnostic::error(
            codes::INPUT_IO,
            &file,
            Default::default(),
            format!("cannot read input: {error}"),
        )]
    })?;

    match request.emit {
        EmitKind::Check => {
            compile_request_to_hir(request, &file, &source, warnings)?;
            Ok(None)
        }
        EmitKind::Hir => {
            let hir = compile_request_to_hir(request, &file, &source, warnings)?;
            write_json(request, &hir)
        }
        EmitKind::Mir => {
            let mir = compile_request_to_mir(request, &file, &source, warnings)?;
            write_json(request, &mir)
        }
        EmitKind::MirText => {
            let mir = compile_request_to_mir(request, &file, &source, warnings)?;
            write_text(request, &mir_print::render_module(&mir))
        }
        EmitKind::Assembly => {
            let assembly = compile_request_to_assembly(request, &file, &source, warnings)?;
            let output = request
                .output
                .clone()
                .unwrap_or_else(|| request.input.with_extension("s"));
            write_output(&file, &output, assembly.as_bytes())?;
            Ok(Some(output))
        }
        EmitKind::Object | EmitKind::Executable => {
            native_artifact(request, &file, &source, warnings)
        }
    }
}

/// One request's front end: what it is compiling, and what it has to say
/// about it (`D-122`).
///
/// The pieces travel together because everything after the front end wants
/// more than the program — the codegen module to partition on, the source map
/// to build line tables from — and reading a package's inputs twice to get
/// them back would read every dependency's sources twice.
struct Analyzed {
    name: String,
    program: TypedProgram,
    options: CompileOptions,
    input: Option<package::PackageInput>,
}

fn analyze_request(
    request: &CompileRequest,
    file: &str,
    source: &str,
    warnings: &mut Vec<Diagnostic>,
) -> CompileResult<Analyzed> {
    let options = request_options(request);
    let input = package_input(request)?;
    let (name, program, diagnostics) = match &input {
        Some(input) => {
            let analysis = package::analyze_package(input, &options);
            (input.name.clone(), analysis.program, analysis.diagnostics)
        }
        None => {
            let analysis = analysis::analyze_source(file, source, &options);
            (file.to_owned(), analysis.program, analysis.diagnostics)
        }
    };
    let Some(program) = program else {
        return Err(diagnostics);
    };
    // A compilation that succeeded has diagnostics exactly when it has
    // warnings, and which of them this run reports was decided in
    // `package::analyze_package`.
    warnings.extend(diagnostics);
    Ok(Analyzed {
        name,
        program,
        options,
        input,
    })
}

fn compile_request_to_hir(
    request: &CompileRequest,
    file: &str,
    source: &str,
    warnings: &mut Vec<Diagnostic>,
) -> CompileResult<TypedProgram> {
    Ok(analyze_request(request, file, source, warnings)?.program)
}

fn compile_request_to_mir(
    request: &CompileRequest,
    file: &str,
    source: &str,
    warnings: &mut Vec<Diagnostic>,
) -> CompileResult<MirModule> {
    let analyzed = analyze_request(request, file, source, warnings)?;
    lower_and_optimize(
        &analyzed.name,
        &analyzed.program,
        &analyzed.options,
        analyzed.input.as_ref(),
    )
}

fn compile_request_to_object(
    request: &CompileRequest,
    file: &str,
    source: &str,
    warnings: &mut Vec<Diagnostic>,
) -> CompileResult<Vec<u8>> {
    let analyzed = analyze_request(request, file, source, warnings)?;
    let module = lower_and_optimize(
        &analyzed.name,
        &analyzed.program,
        &analyzed.options,
        analyzed.input.as_ref(),
    )?;
    codegen::emit_object(
        &analyzed.name,
        &module,
        &codegen_options(
            &analyzed.name,
            &analyzed.options,
            analyzed.input.as_ref(),
            false,
        ),
    )
}

fn compile_request_to_assembly(
    request: &CompileRequest,
    file: &str,
    source: &str,
    warnings: &mut Vec<Diagnostic>,
) -> CompileResult<String> {
    let analyzed = analyze_request(request, file, source, warnings)?;
    let module = lower_and_optimize(
        &analyzed.name,
        &analyzed.program,
        &analyzed.options,
        analyzed.input.as_ref(),
    )?;
    codegen::emit_assembly(
        &analyzed.name,
        &module,
        &codegen_options(
            &analyzed.name,
            &analyzed.options,
            analyzed.input.as_ref(),
            true,
        ),
    )
}

fn request_options(request: &CompileRequest) -> CompileOptions {
    let mut options = request.options.clone();
    // The manager sends these explicitly for a package it resolved; a lone file
    // has no manifest to send them from, and both cases mean the same library,
    // so the defaults come from the library itself (`D-076`). Exactly one
    // direct dependency declares the items (`D-041`), so the first bundled
    // package that has any is the one that supplies them (`D-082`).
    let declared = request
        .toolchain_dependencies
        .iter()
        .filter_map(|namespace| toolchain_package_named(namespace))
        .find(|package| !package.language_items.is_empty());
    if let Some(package) = declared {
        for (name, path) in slopium_std::language_items_of(package.name) {
            let slot = match name.as_str() {
                "option" => &mut options.language_items.option,
                "result" => &mut options.language_items.result,
                "result-ok" => &mut options.language_items.result_ok,
                "result-err" => &mut options.language_items.result_err,
                _ => continue,
            };
            slot.get_or_insert(path);
        }
    }
    options
}

/// The bundled package a toolchain dependency names, if there is one.
///
/// The manager may hand over a nested namespace, so it is the last segment that
/// names the package.
pub fn toolchain_package_named(namespace: &str) -> Option<&'static slopium_std::ToolchainPackage> {
    namespace
        .rsplit(':')
        .next()
        .and_then(slopium_std::toolchain_package)
}

/// The bundled library as package sources under `namespace`: the package that
/// namespace names, and everything it depends on, dependencies first.
///
/// The library ships inside the compiler, so there is no path to read it from
/// and no version to resolve: a build that asked for a toolchain library gets
/// these, and the manager hashes the same bytes into the lock (`D-076`). A root
/// that asks for `std` gets `core` without naming it — that dependency is the
/// library's, not the program's (`D-082`).
pub fn std_package_sources(namespace: &str) -> Vec<package::PackageSource> {
    let Some(package) = toolchain_package_named(namespace) else {
        return Vec::new();
    };
    let mut files: Vec<package::PackageSource> = Vec::new();
    for dependency in package.dependencies {
        for source in std_package_sources(dependency) {
            if !files.iter().any(|existing| {
                existing.namespace == source.namespace && existing.module == source.module
            }) {
                files.push(source);
            }
        }
    }
    files.extend(
        package
            .modules
            .iter()
            .map(|(module, source)| package::PackageSource {
                path: format!("<toolchain:{namespace}>/{module}.slp"),
                namespace: Some(namespace.to_owned()),
                module: (*module).into(),
                source: (*source).into(),
            }),
    );
    files
}

fn package_input(request: &CompileRequest) -> CompileResult<Option<package::PackageInput>> {
    let entry = request.input.canonicalize().map_err(|error| {
        vec![Diagnostic::error(
            codes::INPUT_IO,
            request.input.display().to_string(),
            Default::default(),
            format!("cannot resolve entry source: {error}"),
        )]
    })?;
    let mut files = Vec::new();
    let mut entry_module = None;
    // A lone file is a package of one module, named after the file, whether or
    // not it asked for the library — so `--no-std` changes what a program can
    // call and never what its symbols are called (`D-077`).
    let root = match request.source_root.as_ref() {
        Some(root) => root.canonicalize().map_err(|error| {
            vec![Diagnostic::error(
                codes::INPUT_IO,
                root.display().to_string(),
                Default::default(),
                format!("cannot resolve source root: {error}"),
            )]
        })?,
        None => {
            let directory = entry.parent().unwrap_or(Path::new(".")).to_path_buf();
            let module = module_from_path(&directory, &entry).ok_or_else(|| {
                vec![Diagnostic::error(
                    codes::MODULE,
                    request.input.display().to_string(),
                    Default::default(),
                    "source path cannot be mapped to a module",
                )]
            })?;
            entry_module = Some(module.clone());
            files.push(package::PackageSource {
                path: request.input.display().to_string(),
                namespace: None,
                module,
                source: fs::read_to_string(&entry).map_err(|error| {
                    vec![Diagnostic::error(
                        codes::INPUT_IO,
                        request.input.display().to_string(),
                        Default::default(),
                        format!("cannot read input: {error}"),
                    )]
                })?,
            });
            directory
        }
    };
    let mut paths = Vec::new();
    if request.source_root.is_some() {
        collect_sources(&root, &mut paths).map_err(|error| {
            vec![Diagnostic::error(
                codes::INPUT_IO,
                root.display().to_string(),
                Default::default(),
                format!("cannot discover package sources: {error}"),
            )]
        })?;
        paths.sort();
    }
    for path in paths {
        // A file the manifest named is that module; a path names every other
        // one (`D-009`, `D-135`).
        let module = match request
            .named_modules
            .iter()
            .find(|(_, named)| *named == path)
        {
            Some((name, _)) => name.clone(),
            None => module_from_path(&root, &path).ok_or_else(|| {
                vec![Diagnostic::error(
                    codes::MODULE,
                    path.display().to_string(),
                    Default::default(),
                    "source path cannot be mapped to a module",
                )]
            })?,
        };
        if path == entry {
            entry_module = Some(module.clone());
        }
        // Whoever read the manifest decided this module is not part of a build
        // for the selected target (`D-135`). The entry is never excluded — a
        // package whose entry belongs to one target has nothing to build at
        // all — so the walk keeps it and the check below reports it.
        if request.excluded_modules.contains(&module) && path != entry {
            continue;
        }
        let source = fs::read_to_string(&path).map_err(|error| {
            vec![Diagnostic::error(
                codes::INPUT_IO,
                path.display().to_string(),
                Default::default(),
                format!("cannot read input: {error}"),
            )]
        })?;
        files.push(package::PackageSource {
            path: path.display().to_string(),
            namespace: None,
            module,
            source,
        });
    }
    for dependency in &request.dependencies {
        let dependency_root = dependency.source_root.canonicalize().map_err(|error| {
            vec![Diagnostic::error(
                codes::DEPENDENCY,
                dependency.source_root.display().to_string(),
                Default::default(),
                format!(
                    "cannot resolve dependency `{}` source root: {error}",
                    dependency.namespace
                ),
            )]
        })?;
        let mut dependency_paths = Vec::new();
        collect_sources(&dependency_root, &mut dependency_paths).map_err(|error| {
            vec![Diagnostic::error(
                codes::DEPENDENCY,
                dependency_root.display().to_string(),
                Default::default(),
                format!(
                    "cannot discover dependency `{}` sources: {error}",
                    dependency.namespace
                ),
            )]
        })?;
        dependency_paths.sort();
        for path in dependency_paths {
            let module = match dependency
                .named_modules
                .iter()
                .find(|(_, named)| *named == path)
            {
                Some((name, _)) => name.clone(),
                None => module_from_path(&dependency_root, &path).ok_or_else(|| {
                    vec![Diagnostic::error(
                        codes::DEPENDENCY,
                        path.display().to_string(),
                        Default::default(),
                        "dependency source path cannot be mapped to a module",
                    )]
                })?,
            };
            if dependency.excluded_modules.contains(&module) {
                continue;
            }
            let source = fs::read_to_string(&path).map_err(|error| {
                vec![Diagnostic::error(
                    codes::INPUT_IO,
                    path.display().to_string(),
                    Default::default(),
                    format!("cannot read dependency input: {error}"),
                )]
            })?;
            files.push(package::PackageSource {
                path: path.display().to_string(),
                namespace: Some(dependency.namespace.clone()),
                module,
                source,
            });
        }
    }
    for namespace in &request.toolchain_dependencies {
        if toolchain_package_named(namespace).is_none() {
            return Err(vec![Diagnostic::error(
                codes::DEPENDENCY,
                request.input.display().to_string(),
                Default::default(),
                format!("toolchain dependency `{namespace}` is not available"),
            )]);
        }
        // A package pulls in what it depends on, and the manager also lists
        // those dependencies itself, so the same module can arrive twice.
        for source in std_package_sources(namespace) {
            if !files.iter().any(|existing| {
                existing.namespace == source.namespace && existing.module == source.module
            }) {
                files.push(source);
            }
        }
    }
    let entry_module = entry_module.ok_or_else(|| {
        vec![Diagnostic::error(
            codes::MODULE,
            entry.display().to_string(),
            Default::default(),
            "entry source is outside the source root or is not a `.slp` file",
        )]
    })?;
    let name = match request.source_root.as_ref() {
        Some(_) => root
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or("package")
            .to_owned(),
        None => entry_module.clone(),
    };
    Ok(Some(package::PackageInput {
        name,
        entry_module,
        files,
    }))
}

/// Deepest directory nesting any source walk will descend.
pub const MAX_SOURCE_TREE_DEPTH: usize = 64;

/// Collect `.slp` files under `directory`.
///
/// Directory symlinks are not followed: `entry.file_type()` reports the link
/// itself, so a `src/loop -> ..` cycle cannot spin forever. Depth is capped as
/// a second backstop against pathologically nested trees. Returned paths are
/// canonicalized, so a *file* symlink still resolves to its target.
pub fn collect_slp_sources(directory: &Path, output: &mut Vec<PathBuf>) -> std::io::Result<()> {
    fn walk(directory: &Path, depth: usize, output: &mut Vec<PathBuf>) -> std::io::Result<()> {
        if depth > MAX_SOURCE_TREE_DEPTH {
            return Err(std::io::Error::other(format!(
                "source tree is nested deeper than {MAX_SOURCE_TREE_DEPTH} directories at `{}`",
                directory.display()
            )));
        }
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                walk(&path, depth + 1, output)?;
            } else if path.extension().and_then(|extension| extension.to_str()) == Some("slp") {
                output.push(path.canonicalize()?);
            }
        }
        Ok(())
    }
    walk(directory, 0, output)
}

fn collect_sources(directory: &Path, output: &mut Vec<PathBuf>) -> std::io::Result<()> {
    collect_slp_sources(directory, output)
}

/// The module name a source file has under `root`, path components joined with
/// `:` and the extension dropped (`D-009`).
///
/// Public because the manager asks the same question of a file a manifest
/// names: a `[target."<triple>"]` table is written in paths, because that is
/// what a person edits, and reaches the compiler as module names, because that
/// is what it knows (`D-135`). Two spellings of one rule is how they drift.
pub fn module_from_path(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut parts = relative
        .components()
        .map(|component| component.as_os_str().to_str().map(str::to_owned))
        .collect::<Option<Vec<_>>>()?;
    let last = parts.last_mut()?;
    *last = Path::new(last).file_stem()?.to_str()?.to_owned();
    Some(parts.join(":"))
}

fn write_json<T: Serialize>(request: &CompileRequest, value: &T) -> CompileResult<Option<PathBuf>> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|error| {
        vec![Diagnostic::error(
            codes::OUTPUT_IO,
            request.input.display().to_string(),
            Default::default(),
            format!("cannot serialize compiler IR: {error}"),
        )]
    })?;
    if let Some(output) = &request.output {
        write_output(&request.input.display().to_string(), output, &bytes)?;
        Ok(Some(output.clone()))
    } else {
        println!("{}", String::from_utf8_lossy(&bytes));
        Ok(None)
    }
}

fn write_text(request: &CompileRequest, text: &str) -> CompileResult<Option<PathBuf>> {
    if let Some(output) = &request.output {
        write_output(
            &request.input.display().to_string(),
            output,
            text.as_bytes(),
        )?;
        Ok(Some(output.clone()))
    } else {
        print!("{text}");
        Ok(None)
    }
}

/// Whether this request's object is written by the compiler or by `as`.
///
/// Two things send it back to the assembler. Debug information is one: line
/// tables are built from the `.file` and `.loc` directives, and the object
/// writer emits no DWARF (`D-028`). The other is `SLOPIUM_OBJECT_WRITER`,
/// which exists so that a bug in the encoder has a way around it that does not
/// involve a different compiler.
fn writes_its_own_object(request: &CompileRequest) -> bool {
    if request.options.debug {
        return false;
    }
    if std::env::var("SLOPIUM_OBJECT_WRITER").is_ok_and(|value| value == "external") {
        return false;
    }
    codegen::writes_objects(&request.options.target)
}

fn native_artifact(
    request: &CompileRequest,
    file: &str,
    source: &str,
    warnings: &mut Vec<Diagnostic>,
) -> CompileResult<Option<PathBuf>> {
    let output = request
        .output
        .clone()
        .unwrap_or_else(|| match request.emit {
            EmitKind::Object => request.input.with_extension("o"),
            _ => request.input.with_extension("out"),
        });
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| io_diagnostic(file, "create output directory", error))?;
    }
    // Intermediates must not live at paths derived from `-o`: those are
    // predictable, and a pre-created symlink there would redirect our write, or
    // let someone swap the C runtime we are about to hand to `cc`.
    let scratch = Scratch::new(file)?;
    let internal = writes_its_own_object(request);
    let input_path = if internal {
        let object = compile_request_to_object(request, file, source, warnings)?;
        if request.emit == EmitKind::Object {
            write_output(file, &output, &object)?;
            return Ok(Some(output));
        }
        let path = scratch.path().join("program.o");
        write_new(file, &path, &object)?;
        path
    } else {
        let assembly = compile_request_to_assembly(request, file, source, warnings)?;
        let path = scratch.path().join("program.s");
        write_new(file, &path, assembly.as_bytes())?;
        path
    };

    let mut command = Command::new(&request.cc);
    command.arg("-o").arg(&output);
    if request.emit == EmitKind::Object {
        command.arg("-c").arg(&input_path);
    } else {
        let environment = request_environment(&request.options);
        let mut generated = Vec::new();
        let runtimes: &[PathBuf] = if request.runtimes.is_empty() {
            for (name, bytes) in runtime_sources(environment) {
                let path = scratch.path().join(name);
                write_new(file, &path, bytes)?;
                generated.push(path);
            }
            &generated
        } else {
            &request.runtimes
        };
        // The runtime is C files of every helper the language might call, and
        // most programs call a handful. The shared flags put each function in
        // its own section and let the linker drop what nothing reaches, then
        // strip the symbol table when the caller asked for it.
        command.args(cc_flags(
            environment,
            request.options.strip,
            request.options.panic_abort,
        ));
        command.arg(&input_path).args(runtimes);
    }
    let result = command.output().map_err(|error| {
        vec![Diagnostic::error(
            codes::TOOLCHAIN,
            file,
            Default::default(),
            format!("failed to run `{}`: {error}", request.cc),
        )
        .with_help("install a target C toolchain or configure another `cc`")]
    })?;
    if !result.status.success() {
        return Err(vec![Diagnostic::error(
            codes::TOOLCHAIN,
            file,
            Default::default(),
            format!(
                "assembler/linker failed:\n{}",
                String::from_utf8_lossy(&result.stderr).trim()
            ),
        )]);
    }
    Ok(Some(output))
}

/// A private directory for build intermediates, removed when it goes out of
/// scope. `create_dir` fails rather than following an existing entry, so the
/// name cannot be squatted, and 0o700 keeps the contents unreadable to others.
struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(file: &str) -> CompileResult<Self> {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let base = std::env::temp_dir();
        let mut last = None;
        for _ in 0..64 {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default();
            let sequence = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = base.join(format!(
                "slopic-{}-{nonce:x}-{sequence:x}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => {
                    restrict_permissions(&path);
                    return Ok(Self { path });
                }
                Err(error) => last = Some(error),
            }
        }
        Err(io_diagnostic(
            file,
            "create a private build directory",
            last.unwrap_or_else(|| std::io::Error::other("no attempt succeeded")),
        ))
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

/// Write a file that must not already exist, so an attacker-placed symlink is
/// refused instead of followed.
fn write_new(file: &str, output: &Path, bytes: &[u8]) -> CompileResult<()> {
    use std::io::Write;
    let mut handle = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output)
        .map_err(|error| io_diagnostic(file, "create build intermediate", error))?;
    handle
        .write_all(bytes)
        .map_err(|error| io_diagnostic(file, "write build intermediate", error))
}

fn write_output(file: &str, output: &Path, bytes: &[u8]) -> CompileResult<()> {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| io_diagnostic(file, "create output directory", error))?;
    }
    fs::write(output, bytes).map_err(|error| io_diagnostic(file, "write output", error))
}

fn io_diagnostic(file: &str, action: &str, error: std::io::Error) -> Vec<Diagnostic> {
    vec![Diagnostic::error(
        codes::OUTPUT_IO,
        file,
        Default::default(),
        format!("cannot {action}: {error}"),
    )]
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    #[test]
    fn invalid_source_returns_diagnostic_not_panic() {
        let errors = compile_to_hir("bad.slp", "(fn main", &CompileOptions::default()).unwrap_err();
        assert!(!errors.is_empty());
    }

    #[test]
    fn deeply_nested_input_is_rejected_without_exhausting_the_stack() {
        // Every pass over the tree recurses, including the lossless syntax
        // tree's own drop, so this exercises the whole front end - not just
        // the parser - on input that used to abort the process.
        for source in [
            "(".repeat(500_000),
            format!("{}{}", "(".repeat(500_000), ")".repeat(500_000)),
            format!("(fn main () -> i32 {} 0)", "(".repeat(500_000)),
        ] {
            let analysis =
                analysis::analyze_source("deep.slp", &source, &CompileOptions::default());
            assert!(
                !analysis.diagnostics.is_empty(),
                "deep nesting must be diagnosed"
            );
            assert!(analysis.program.is_none());
        }
    }

    #[test]
    fn malformed_input_corpus_never_panics() {
        let seeds = [
            "",
            "(",
            ")",
            "\"",
            "(fn)",
            "(fn main () -> i32)",
            "(fn main ((x)) -> nope x)",
            "(match",
            "(enum E (V (",
            "(fn main () -> i32 (let x \"💥\") (+ x 1))",
        ];
        let options = CompileOptions::default();
        for seed in seeds {
            for end in seed
                .char_indices()
                .map(|(index, _)| index)
                .chain(std::iter::once(seed.len()))
            {
                let source = &seed[..end];
                let result =
                    std::panic::catch_unwind(|| compile_to_hir("fuzz.slp", source, &options));
                assert!(result.is_ok(), "compiler panicked for {source:?}");
            }
        }
    }
}
