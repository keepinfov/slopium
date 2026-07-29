pub mod analysis;
pub mod ast;
pub mod cfg;
pub mod codegen;
pub mod diagnostic;
pub mod lexer;
pub mod mir;
pub mod mir_print;
pub mod package;
pub mod parser;
pub mod sema;
pub mod syntax;
pub mod verify;

use crate::codegen::{CodegenOptions, SUPPORTED_TARGET};
use crate::diagnostic::{codes, CompileResult, Diagnostic};
use crate::mir::MirModule;
use crate::sema::TypedProgram;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const COMPILER_PROTOCOL: u32 = 2;
pub const RUNTIME_SOURCE: &[u8] = include_bytes!("../../../runtime/slop_rt.c");
pub const STANDARD_LIBRARY_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug)]
pub struct CompileOptions {
    pub target: String,
    pub test_harness: bool,
    pub optimize: bool,
    pub codegen_module: Option<String>,
    pub language_items: LanguageItems,
    pub validate_entry_point: bool,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            target: SUPPORTED_TARGET.into(),
            test_harness: false,
            optimize: false,
            codegen_module: None,
            language_items: LanguageItems::default(),
            validate_entry_point: true,
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
    pub dependencies: Vec<DependencySource>,
    pub toolchain_dependencies: Vec<String>,
    pub output: Option<PathBuf>,
    pub emit: EmitKind,
    pub options: CompileOptions,
    pub runtime: Option<PathBuf>,
    pub cc: String,
}

#[derive(Clone, Debug)]
pub struct DependencySource {
    pub namespace: String,
    pub source_root: PathBuf,
}

#[derive(Clone, Debug, Serialize)]
pub struct CompilerInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub protocol: u32,
    pub targets: &'static [&'static str],
    pub standard_library: &'static str,
}

pub fn compiler_info() -> CompilerInfo {
    CompilerInfo {
        name: "slopic",
        version: env!("CARGO_PKG_VERSION"),
        protocol: COMPILER_PROTOCOL,
        targets: &[SUPPORTED_TARGET],
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
    let program = compile_to_hir(file, source, options)?;
    let mut module = mir::lower(&program);
    verify::check(file, &module, "lowering")?;
    if options.optimize {
        mir::optimize(&mut module);
        verify::check(file, &module, "constant folding")?;
    }
    Ok(module)
}

pub fn compile_package_to_mir(
    input: &package::PackageInput,
    options: &CompileOptions,
) -> CompileResult<MirModule> {
    let program = package::compile_package_to_hir(input, options)?;
    let mut module = mir::lower(&program);
    verify::check(&input.name, &module, "lowering")?;
    // `partition_codegen` only flips `emit` flags, so it cannot invalidate any
    // verified invariant and is not worth a second pass — this runs once per
    // owner module per build.
    if let Some(selected) = &options.codegen_module {
        partition_codegen(&mut module, input, selected);
    }
    if options.optimize {
        mir::optimize(&mut module);
        verify::check(&input.name, &module, "constant folding")?;
    }
    Ok(module)
}

pub fn compile_to_assembly(
    file: &str,
    source: &str,
    options: &CompileOptions,
) -> CompileResult<String> {
    let mir = compile_to_mir(file, source, options)?;
    codegen::emit_assembly(
        file,
        &mir,
        &CodegenOptions {
            target: options.target.clone(),
            test_harness: options.test_harness,
            emit_entrypoint: true,
        },
    )
}

pub fn compile_package_to_assembly(
    input: &package::PackageInput,
    options: &CompileOptions,
) -> CompileResult<String> {
    let mir = compile_package_to_mir(input, options)?;
    let emits_entrypoint = options
        .codegen_module
        .as_ref()
        .is_none_or(|module| module == &input.entry_module);
    codegen::emit_assembly(
        &input.name,
        &mir,
        &CodegenOptions {
            target: options.target.clone(),
            test_harness: options.test_harness && emits_entrypoint,
            emit_entrypoint: emits_entrypoint,
        },
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
    for function in &mut module.functions {
        function.emit = owner(&function.name) == Some(selected);
    }
    for test in &mut module.tests {
        test.emit = owner(&test.name) == Some(selected);
        test.function.emit = test.emit;
    }
    for structure in &mut module.structs {
        structure.emit = owner(&structure.name) == Some(selected);
    }
    for enumeration in &mut module.enums {
        enumeration.emit = owner(&enumeration.name) == Some(selected);
    }
}

pub fn compile(request: &CompileRequest) -> CompileResult<Option<PathBuf>> {
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
            compile_request_to_hir(request, &file, &source)?;
            Ok(None)
        }
        EmitKind::Hir => {
            let hir = compile_request_to_hir(request, &file, &source)?;
            write_json(request, &hir)
        }
        EmitKind::Mir => {
            let mir = compile_request_to_mir(request, &file, &source)?;
            write_json(request, &mir)
        }
        EmitKind::MirText => {
            let mir = compile_request_to_mir(request, &file, &source)?;
            write_text(request, &mir_print::render_module(&mir))
        }
        EmitKind::Assembly => {
            let assembly = compile_request_to_assembly(request, &file, &source)?;
            let output = request
                .output
                .clone()
                .unwrap_or_else(|| request.input.with_extension("s"));
            write_output(&file, &output, assembly.as_bytes())?;
            Ok(Some(output))
        }
        EmitKind::Object | EmitKind::Executable => native_artifact(request, &file, &source),
    }
}

fn compile_request_to_hir(
    request: &CompileRequest,
    file: &str,
    source: &str,
) -> CompileResult<TypedProgram> {
    let options = request_options(request);
    match package_input(request)? {
        Some(input) => package::compile_package_to_hir(&input, &options),
        None => compile_to_hir(file, source, &options),
    }
}

fn compile_request_to_mir(
    request: &CompileRequest,
    file: &str,
    source: &str,
) -> CompileResult<MirModule> {
    let options = request_options(request);
    match package_input(request)? {
        Some(input) => compile_package_to_mir(&input, &options),
        None => compile_to_mir(file, source, &options),
    }
}

fn compile_request_to_assembly(
    request: &CompileRequest,
    file: &str,
    source: &str,
) -> CompileResult<String> {
    let options = request_options(request);
    match package_input(request)? {
        Some(input) => compile_package_to_assembly(&input, &options),
        None => compile_to_assembly(file, source, &options),
    }
}

fn request_options(request: &CompileRequest) -> CompileOptions {
    let mut options = request.options.clone();
    if request
        .toolchain_dependencies
        .iter()
        .any(|namespace| namespace == "std")
    {
        options
            .language_items
            .option
            .get_or_insert_with(|| "std:option:Option".into());
        options
            .language_items
            .result
            .get_or_insert_with(|| "std:result:Result".into());
        options
            .language_items
            .result_ok
            .get_or_insert_with(|| "std:result:Ok".into());
        options
            .language_items
            .result_err
            .get_or_insert_with(|| "std:result:Err".into());
    }
    options
}

fn package_input(request: &CompileRequest) -> CompileResult<Option<package::PackageInput>> {
    let Some(root) = request.source_root.as_ref() else {
        return Ok(None);
    };
    let root = root.canonicalize().map_err(|error| {
        vec![Diagnostic::error(
            codes::INPUT_IO,
            root.display().to_string(),
            Default::default(),
            format!("cannot resolve source root: {error}"),
        )]
    })?;
    let entry = request.input.canonicalize().map_err(|error| {
        vec![Diagnostic::error(
            codes::INPUT_IO,
            request.input.display().to_string(),
            Default::default(),
            format!("cannot resolve entry source: {error}"),
        )]
    })?;
    let mut paths = Vec::new();
    collect_sources(&root, &mut paths).map_err(|error| {
        vec![Diagnostic::error(
            codes::INPUT_IO,
            root.display().to_string(),
            Default::default(),
            format!("cannot discover package sources: {error}"),
        )]
    })?;
    paths.sort();
    let mut files = Vec::new();
    let mut entry_module = None;
    for path in paths {
        let module = module_from_path(&root, &path).ok_or_else(|| {
            vec![Diagnostic::error(
                codes::MODULE,
                path.display().to_string(),
                Default::default(),
                "source path cannot be mapped to a module",
            )]
        })?;
        if path == entry {
            entry_module = Some(module.clone());
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
            let module = module_from_path(&dependency_root, &path).ok_or_else(|| {
                vec![Diagnostic::error(
                    codes::DEPENDENCY,
                    path.display().to_string(),
                    Default::default(),
                    "dependency source path cannot be mapped to a module",
                )]
            })?;
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
        if namespace.rsplit(':').next() != Some("std") {
            return Err(vec![Diagnostic::error(
                codes::DEPENDENCY,
                request.input.display().to_string(),
                Default::default(),
                format!("toolchain dependency `{namespace}` is not available"),
            )]);
        }
        for (module, source) in [
            (
                "option",
                "(export Option)\n(enum Option (T) None (Some ((value T))))\n",
            ),
            (
                "result",
                "(export Result (Result:Ok :as Ok) (Result:Err :as Err))\n\
                 (enum Result (T E)\n\
                   (Ok ((value T)))\n\
                   (Err ((error E))))\n",
            ),
        ] {
            files.push(package::PackageSource {
                path: format!("<toolchain:{namespace}>/{module}.slp"),
                namespace: Some(namespace.clone()),
                module: module.into(),
                source: source.into(),
            });
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
    let name = root
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("package")
        .to_owned();
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

fn module_from_path(root: &Path, path: &Path) -> Option<String> {
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

fn native_artifact(
    request: &CompileRequest,
    file: &str,
    source: &str,
) -> CompileResult<Option<PathBuf>> {
    let assembly = compile_request_to_assembly(request, file, source)?;
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
    let assembly_path = scratch.path().join("program.s");
    write_new(file, &assembly_path, assembly.as_bytes())?;

    let mut command = Command::new(&request.cc);
    command.arg("-o").arg(&output);
    if request.emit == EmitKind::Object {
        command.arg("-c").arg(&assembly_path);
    } else {
        let generated;
        let runtime = if let Some(runtime) = request.runtime.as_ref() {
            runtime
        } else {
            generated = scratch.path().join("slop_rt.c");
            write_new(file, &generated, RUNTIME_SOURCE)?;
            &generated
        };
        command.arg(&assembly_path).arg(runtime);
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
