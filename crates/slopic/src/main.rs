use clap::{Parser, ValueEnum};
use slopic_core::diagnostic::Diagnostic;
use slopic_core::{
    compile, compiler_info, CompileOptions, CompileRequest, DependencySource, EmitKind,
    LanguageItems, COMPILER_PROTOCOL,
};
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

    #[arg(long = "language-item", value_name = "NAME=PATH")]
    language_items: Vec<String>,

    #[arg(long, value_enum, default_value = "exe")]
    emit: Emit,

    #[arg(short, long)]
    output: Option<PathBuf>,

    #[arg(long, default_value = "x86_64-unknown-linux-gnu")]
    target: String,

    #[arg(long)]
    test: bool,

    #[arg(long)]
    codegen_module: Option<String>,

    #[arg(long, value_enum, default_value = "dev")]
    profile: Profile,

    /// Emit DWARF line tables so a debugger can map addresses to source.
    #[arg(long)]
    debug: bool,

    #[arg(long)]
    runtime: Option<PathBuf>,

    #[arg(long, default_value = "cc")]
    cc: String,

    #[arg(long, value_enum, default_value = "human")]
    diagnostic_format: DiagnosticFormat,

    #[arg(long)]
    info: bool,
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

#[derive(Clone, Copy, ValueEnum)]
enum Profile {
    Dev,
    Release,
}

fn main() {
    let cli = Cli::parse();
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
    if cli
        .toolchain_dependencies
        .iter()
        .any(|namespace| namespace == "std")
    {
        language_items
            .option
            .get_or_insert_with(|| "std:option:Option".into());
        language_items
            .result
            .get_or_insert_with(|| "std:result:Result".into());
        language_items
            .result_ok
            .get_or_insert_with(|| "std:result:Ok".into());
        language_items
            .result_err
            .get_or_insert_with(|| "std:result:Err".into());
    }
    let request = CompileRequest {
        input,
        source_root: cli.source_root,
        dependencies,
        toolchain_dependencies: cli.toolchain_dependencies,
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
        options: CompileOptions {
            target: cli.target,
            test_harness: cli.test,
            optimize: matches!(cli.profile, Profile::Release),
            codegen_module: cli.codegen_module,
            language_items,
            validate_entry_point: true,
            debug: cli.debug,
        },
        runtime: cli.runtime,
        cc: cli.cc,
    };
    if let Err(diagnostics) = compile(&request) {
        for diagnostic in diagnostics {
            let diagnostic_source = if diagnostic.file == input_name {
                source.clone()
            } else {
                std::fs::read_to_string(&diagnostic.file).unwrap_or_default()
            };
            render(&diagnostic, &diagnostic_source, cli.diagnostic_format);
        }
        std::process::exit(1);
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
