use clap::{CommandFactory, Parser, ValueEnum};
use clap_complete::{generate, Shell};
use slopic_core::diagnostic::Diagnostic;
use slopic_core::{
    compile, compiler_info, CompileOptions, CompileRequest, DependencySource, EmitKind,
    LanguageItems, COMPILER_PROTOCOL,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "slopic", version, about = "Internal Slopium native compiler")]
struct Cli {
    input: Option<PathBuf>,

    #[arg(long)]
    source_root: Option<PathBuf>,

    #[arg(long = "dependency", value_name = "ALIAS=SOURCE_ROOT")]
    dependencies: Vec<String>,

    #[arg(long = "toolchain-dependency", value_name = "ALIAS")]
    toolchain_dependencies: Vec<String>,

    /// A module of the package under `--source-root` to leave out of the build.
    ///
    /// The compiler discovers a package's modules by walking its source root
    /// and has no manifest to consult (`D-002`), so a package that names a
    /// module per target is resolved by the manager, which says here which ones
    /// this build is not made of (`D-135`).
    #[arg(long = "exclude-module", value_name = "MODULE")]
    exclude_modules: Vec<String>,

    /// The same, for a module of a dependency.
    ///
    /// Named by alias rather than qualified into one string, because a module
    /// name already holds colons and `a:b:c` would not say which part is the
    /// alias.
    #[arg(long = "dependency-exclude", value_name = "ALIAS=MODULE")]
    dependency_excludes: Vec<String>,

    /// The module name a source file has, overriding the one its path would
    /// give it.
    ///
    /// A manifest may say what a module *is* per target, so that a program can
    /// name one module and get a different file depending on what is being
    /// built (`D-135`). Path derivation still names every file nobody said
    /// anything about.
    #[arg(long = "module", value_name = "MODULE=PATH")]
    named_modules: Vec<String>,

    /// The same, for a file of a dependency.
    #[arg(long = "dependency-module", value_name = "ALIAS=MODULE=PATH")]
    dependency_modules: Vec<String>,

    /// Compile a lone file without the bundled library. A file has no manifest
    /// to declare a dependency in, so it gets the library by default (`D-077`);
    /// this is for the object that must carry nothing but its own symbols. A
    /// package is unaffected either way: `--source-root` means the manifest
    /// decided.
    #[arg(long)]
    no_std: bool,

    /// Build for a target with no C library under it: no `main` wrapper, only
    /// the core half of the runtime, and `core` as the default library for a
    /// lone file. The program supplies `sl_rt_alloc`, `sl_rt_free`,
    /// `sl_rt_abort` and `sl_rt_panic` itself (`D-080`, `D-081`).
    #[arg(long)]
    freestanding: bool,

    #[arg(long = "language-item", value_name = "NAME=PATH")]
    language_items: Vec<String>,

    #[arg(long, value_enum, default_value = "exe")]
    emit: Emit,

    #[arg(short, long)]
    output: Option<PathBuf>,

    #[arg(long, default_value = slopic_core::codegen::DEFAULT_TARGET)]
    target: String,

    #[arg(long)]
    test: bool,

    #[arg(long)]
    codegen_module: Option<String>,

    /// Compile a library: no entry point is required, because nothing here is
    /// linked into an executable on its own. `slopium` passes this for a
    /// package entered through `lib.slp` (`D-015`).
    #[arg(long)]
    library: bool,

    /// Run the optimization pipeline. `slopic` knows nothing about "profiles":
    /// the manager decides that a release build optimizes and passes this.
    #[arg(long)]
    optimize: bool,

    /// Emit DWARF line tables so a debugger can map addresses to source.
    #[arg(long)]
    debug: bool,

    /// Abort on a trap without printing a message, and emit no panic strings.
    #[arg(long)]
    panic_abort: bool,

    /// Strip the symbol table from a linked executable. Mutually exclusive in
    /// practice with `--debug`, which needs the symbols it would remove.
    #[arg(long)]
    strip: bool,

    /// Link this C file as the runtime instead of the bundled one. Repeatable:
    /// the runtime is more than one translation unit since `D-066`.
    #[arg(long = "runtime", value_name = "FILE")]
    runtimes: Vec<PathBuf>,

    #[arg(long, default_value = "cc")]
    cc: String,

    #[arg(long, value_enum, default_value = "human")]
    diagnostic_format: DiagnosticFormat,

    #[arg(long)]
    info: bool,

    /// Print a shell completion script for `slopic` on stdout and exit.
    #[arg(long, value_name = "SHELL", value_enum)]
    completions: Option<Shell>,
}

#[derive(Clone, Copy, ValueEnum)]
enum Emit {
    Check,
    Hir,
    Mir,
    MirText,
    Asm,
    Obj,
    Exe,
}

#[derive(Clone, Copy, ValueEnum)]
enum DiagnosticFormat {
    Human,
    Json,
}

/// `MODULE=PATH`, with the path canonicalized so it compares equal to what the
/// source walk found (`D-135`).
fn parse_named_modules(values: &[String]) -> Result<Vec<(String, PathBuf)>, String> {
    values
        .iter()
        .map(|value| {
            let (module, path) = value
                .split_once('=')
                .ok_or_else(|| format!("invalid module `{value}`; expected MODULE=PATH"))?;
            if module.is_empty() || path.is_empty() {
                return Err(format!(
                    "invalid module `{value}`; a name and a path are required"
                ));
            }
            let resolved = PathBuf::from(path).canonicalize().map_err(|error| {
                format!("cannot resolve module `{module}` at `{path}`: {error}")
            })?;
            Ok((module.to_owned(), resolved))
        })
        .collect()
}

fn main() {
    let cli = Cli::parse();
    if let Some(shell) = cli.completions {
        let mut command = Cli::command();
        let name = command.get_name().to_string();
        generate(shell, &mut command, name, &mut std::io::stdout());
        return;
    }
    if cli.info {
        println!(
            "{}",
            serde_json::to_string_pretty(&compiler_info()).expect("compiler info serializes")
        );
        return;
    }
    let Some(input) = cli.input else {
        eprintln!("slopic: input file is required (protocol {COMPILER_PROTOCOL})");
        std::process::exit(2);
    };
    let source = std::fs::read_to_string(&input).unwrap_or_default();
    let input_name = input.display().to_string();
    let named_modules = match parse_named_modules(&cli.named_modules) {
        Ok(named) => named,
        Err(error) => {
            eprintln!("slopic: {error}");
            std::process::exit(2);
        }
    };
    let mut dependency_modules: BTreeMap<String, Vec<(String, PathBuf)>> = BTreeMap::new();
    for value in &cli.dependency_modules {
        let Some((namespace, rest)) = value.split_once('=') else {
            eprintln!("slopic: invalid dependency module `{value}`; expected ALIAS=MODULE=PATH");
            std::process::exit(2);
        };
        if namespace.is_empty() {
            eprintln!("slopic: invalid dependency module `{value}`; an alias is required");
            std::process::exit(2);
        }
        match parse_named_modules(&[rest.to_owned()]) {
            Ok(mut named) => dependency_modules
                .entry(namespace.to_owned())
                .or_default()
                .append(&mut named),
            Err(error) => {
                eprintln!("slopic: {error}");
                std::process::exit(2);
            }
        }
    }
    let mut dependency_excludes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for value in &cli.dependency_excludes {
        let Some((namespace, module)) = value.split_once('=') else {
            eprintln!("slopic: invalid dependency exclusion `{value}`; expected ALIAS=MODULE");
            std::process::exit(2);
        };
        if namespace.is_empty() || module.is_empty() {
            eprintln!(
                "slopic: invalid dependency exclusion `{value}`; alias and module are required"
            );
            std::process::exit(2);
        }
        dependency_excludes
            .entry(namespace.to_owned())
            .or_default()
            .push(module.to_owned());
    }
    let dependencies = match cli
        .dependencies
        .iter()
        .map(|value| {
            let (namespace, path) = value.split_once('=').ok_or_else(|| {
                format!("invalid dependency `{value}`; expected ALIAS=SOURCE_ROOT")
            })?;
            if namespace.is_empty() || path.is_empty() {
                return Err(format!(
                    "invalid dependency `{value}`; alias and source root are required"
                ));
            }
            Ok(DependencySource {
                excluded_modules: dependency_excludes
                    .get(namespace)
                    .cloned()
                    .unwrap_or_default(),
                named_modules: dependency_modules
                    .get(namespace)
                    .cloned()
                    .unwrap_or_default(),
                namespace: namespace.to_owned(),
                source_root: PathBuf::from(path),
            })
        })
        .collect::<Result<Vec<_>, String>>()
    {
        Ok(dependencies) => dependencies,
        Err(error) => {
            eprintln!("slopic: {error}");
            std::process::exit(2);
        }
    };
    let mut language_items = LanguageItems::default();
    for value in &cli.language_items {
        let Some((name, path)) = value.split_once('=') else {
            eprintln!("slopic: invalid language item `{value}`; expected NAME=PATH");
            std::process::exit(2);
        };
        let slot = match name {
            "option" => &mut language_items.option,
            "result" => &mut language_items.result,
            "result-ok" => &mut language_items.result_ok,
            "result-err" => &mut language_items.result_err,
            _ => {
                eprintln!("slopic: unknown language item `{name}`");
                std::process::exit(2);
            }
        };
        *slot = Some(path.to_owned());
    }
    // The language items the bundled library supplies are filled in by the
    // compiler, from the library, for whoever asked for it.
    let options = CompileOptions {
        target: cli.target,
        test_harness: cli.test,
        optimize: cli.optimize,
        codegen_module: cli.codegen_module,
        language_items,
        validate_entry_point: !cli.library,
        debug: cli.debug,
        panic_abort: cli.panic_abort,
        strip: cli.strip,
        environment: cli
            .freestanding
            .then_some(slopic_core::codegen::Environment::Freestanding),
    };
    let mut toolchain_dependencies = cli.toolchain_dependencies;
    if cli.source_root.is_none() && !cli.no_std && toolchain_dependencies.is_empty() {
        // Which library a lone file gets is the environment's to say: a
        // freestanding one has no `std` to offer (`D-081`).
        toolchain_dependencies.push(
            slopic_core::request_environment(&options)
                .default_library()
                .to_owned(),
        );
    }
    let request = CompileRequest {
        input,
        source_root: cli.source_root,
        excluded_modules: cli.exclude_modules,
        named_modules,
        dependencies,
        toolchain_dependencies,
        output: cli.output,
        emit: match cli.emit {
            Emit::Check => EmitKind::Check,
            Emit::Hir => EmitKind::Hir,
            Emit::Mir => EmitKind::Mir,
            Emit::MirText => EmitKind::MirText,
            Emit::Asm => EmitKind::Assembly,
            Emit::Obj => EmitKind::Object,
            Emit::Exe => EmitKind::Executable,
        },
        options,
        runtimes: cli.runtimes,
        cc: cli.cc,
    };
    match compile(&request) {
        // A warning is reported and the compilation still succeeded, which is
        // the whole difference between the two halves here (`D-122`).
        Ok(compiled) => report(
            &compiled.warnings,
            &input_name,
            &source,
            cli.diagnostic_format,
        ),
        Err(diagnostics) => {
            report(&diagnostics, &input_name, &source, cli.diagnostic_format);
            // `1` means the program does not compile and every diagnostic has
            // been printed; everything else here exits `2`. The manager reads
            // that difference to decide whether its own summary line has
            // anything left to say (`D-154`), so the two codes are a contract
            // between the programs rather than an accident of this line.
            std::process::exit(1);
        }
    }
}

fn report(diagnostics: &[Diagnostic], input_name: &str, source: &str, format: DiagnosticFormat) {
    for diagnostic in diagnostics {
        let diagnostic_source = if diagnostic.file == input_name {
            source.to_owned()
        } else {
            std::fs::read_to_string(&diagnostic.file).unwrap_or_default()
        };
        render(diagnostic, &diagnostic_source, format);
    }
}

fn render(diagnostic: &Diagnostic, source: &str, format: DiagnosticFormat) {
    match format {
        DiagnosticFormat::Human => eprintln!("{}", diagnostic.render(source)),
        DiagnosticFormat::Json => {
            eprintln!(
                "{}",
                serde_json::to_string(diagnostic).expect("diagnostic serializes")
            )
        }
    }
}
