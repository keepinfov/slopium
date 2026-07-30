use clap::{Args, Parser, Subcommand};
use serde::Deserialize;
use slopic_core::codegen::SUPPORTED_TARGET;
use slopic_core::syntax::{format_source, FormatOptions};
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs;
use std::fs::OpenOptions;
use std::hash::Hasher;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

#[derive(Parser)]
#[command(name = "slopium", version, about = "Slopium project and build manager")]
struct Cli {
    #[arg(long, global = true)]
    manifest_path: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    New {
        name: String,
        #[arg(long)]
        path: Option<PathBuf>,
    },
    Check(TargetArgs),
    Build(BuildArgs),
    Run {
        #[command(flatten)]
        build: BuildArgs,
        #[arg(last = true)]
        args: Vec<OsString>,
    },
    Test(BuildArgs),
    Fmt {
        #[arg(long)]
        check: bool,
    },
    Clean,
    Targets,
    Compiler,
}

#[derive(Args, Clone)]
struct TargetArgs {
    #[arg(long)]
    target: Option<String>,
    #[arg(long)]
    cc: Option<String>,
}

#[derive(Args, Clone)]
struct BuildArgs {
    #[arg(long)]
    target: Option<String>,
    #[arg(long)]
    release: bool,
    #[arg(long)]
    cc: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    package: Package,
    #[serde(default)]
    dependencies: HashMap<String, DependencySpec>,
    #[serde(default, rename = "language-items")]
    language_items: LanguageItemSection,
    #[serde(default)]
    build: BuildSection,
    #[serde(default)]
    profile: HashMap<String, Profile>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct LanguageItemSection {
    option: Option<String>,
    result: Option<String>,
    #[serde(rename = "result-ok")]
    result_ok: Option<String>,
    #[serde(rename = "result-err")]
    result_err: Option<String>,
}

impl LanguageItemSection {
    fn entries(&self) -> Vec<(String, String)> {
        [
            ("option", self.option.as_ref()),
            ("result", self.result.as_ref()),
            ("result-ok", self.result_ok.as_ref()),
            ("result-err", self.result_err.as_ref()),
        ]
        .into_iter()
        .filter_map(|(name, value)| value.map(|value| (name.to_owned(), value.clone())))
        .collect()
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum DependencySpec {
    Path { path: PathBuf },
    Toolchain { toolchain: bool },
}

#[derive(Debug, Deserialize)]
struct Package {
    name: String,
    version: String,
    entry: PathBuf,
    source: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
struct BuildSection {
    target: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct Profile {
    #[serde(rename = "opt-level")]
    opt_level: Option<u8>,
    debug: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
struct LocalConfig {
    #[serde(default)]
    toolchain: Toolchain,
    #[serde(default)]
    target: HashMap<String, Toolchain>,
}

#[derive(Debug, Default, Deserialize, Clone)]
struct Toolchain {
    cc: Option<String>,
}

#[derive(Deserialize)]
struct CompilerHandshake {
    protocol: u32,
    targets: Vec<String>,
}

struct Project {
    root: PathBuf,
    manifest_path: PathBuf,
    manifest_source: String,
    manifest: Manifest,
    config: LocalConfig,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::New { name, path } => create_project(&name, path),
        Commands::Check(args) => load_project(cli.manifest_path)
            .and_then(|project| check(&project, args.target, args.cc)),
        Commands::Build(args) => load_project(cli.manifest_path)
            .and_then(|project| build(&project, &args, false).map(|_| ())),
        Commands::Run {
            build: args,
            args: program_args,
        } => load_project(cli.manifest_path).and_then(|project| {
            let artifact = build(&project, &args, false)?;
            run_artifact(&artifact, &program_args)
        }),
        Commands::Test(args) => load_project(cli.manifest_path).and_then(|project| {
            let artifact = build(&project, &args, true)?;
            let status = Command::new(&artifact)
                .status()
                .map_err(|error| format!("cannot execute tests: {error}"))?;
            status_result(status, "tests")
        }),
        Commands::Fmt { check } => {
            load_project(cli.manifest_path).and_then(|project| format_project(&project, check))
        }
        Commands::Clean => load_project(cli.manifest_path).and_then(clean),
        Commands::Targets => {
            println!("{SUPPORTED_TARGET} (installed)");
            Ok(())
        }
        Commands::Compiler => compiler_info(),
    };

    if let Err(error) = result {
        eprintln!("slopium: {error}");
        std::process::exit(1);
    }
}

fn format_project(project: &Project, check: bool) -> Result<(), String> {
    let mut differences = Vec::new();
    for source_path in source_files(project)? {
        let source = fs::read_to_string(&source_path)
            .map_err(|error| format!("cannot read `{}`: {error}", source_path.display()))?;
        let formatted = format_source(
            &source_path.display().to_string(),
            &source,
            &FormatOptions::default(),
        )
        .map_err(|diagnostics| {
            diagnostics
                .iter()
                .map(|diagnostic| diagnostic.render(&source))
                .collect::<Vec<_>>()
                .join("\n\n")
        })?;
        if formatted != source {
            if check {
                differences.push(source_path);
            } else {
                atomic_write(&source_path, formatted.as_bytes())?;
                println!("Formatted {}", source_path.display());
            }
        } else {
            println!("Formatted {}", source_path.display());
        }
    }
    if !differences.is_empty() {
        return Err(format!(
            "formatting differs: {}",
            differences
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("`{}` has no parent directory", path.display()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| format!("`{}` has no file name", path.display()))?
        .to_string_lossy();
    let temporary = parent.join(format!(".{file_name}.slopium-fmt-{}", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| {
                format!(
                    "cannot create formatter temporary `{}`: {error}",
                    temporary.display()
                )
            })?;
        file.write_all(bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| format!("cannot write `{}`: {error}", temporary.display()))?;
        if let Ok(metadata) = fs::metadata(path) {
            fs::set_permissions(&temporary, metadata.permissions()).map_err(|error| {
                format!(
                    "cannot preserve permissions for `{}`: {error}",
                    path.display()
                )
            })?;
        }
        fs::rename(&temporary, path)
            .map_err(|error| format!("cannot replace `{}`: {error}", path.display()))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn create_project(name: &str, path: Option<PathBuf>) -> Result<(), String> {
    validate_package_name(name)?;
    let root = path.unwrap_or_else(|| PathBuf::from(name));
    if root.exists() {
        return Err(format!("destination `{}` already exists", root.display()));
    }
    fs::create_dir_all(root.join("src"))
        .map_err(|error| format!("cannot create project: {error}"))?;
    let manifest = format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nsource = \"src\"\nentry = \"src/main.slp\"\n\n\
         [build]\ntarget = \"{SUPPORTED_TARGET}\"\n\n\
         [profile.dev]\nopt-level = 0\ndebug = true\n\n\
         [profile.release]\nopt-level = 1\ndebug = false\n"
    );
    let source = format!(
        "(fn main () -> i32\n  (let message \"hello from {name}\")\n  (println (& message))\n  0)\n\n\
         (test \"arithmetic\"\n  (= (+ 20 22) 42))\n"
    );
    fs::write(root.join("Slopium.toml"), manifest)
        .map_err(|error| format!("cannot write manifest: {error}"))?;
    fs::write(root.join("src/main.slp"), source)
        .map_err(|error| format!("cannot write source: {error}"))?;
    fs::write(root.join(".gitignore"), "/target/\n/.slopium/\n")
        .map_err(|error| format!("cannot write .gitignore: {error}"))?;
    println!("Created package `{name}` at {}", root.display());
    Ok(())
}

fn load_project(manifest_path: Option<PathBuf>) -> Result<Project, String> {
    let manifest_path = match manifest_path {
        Some(path) => path,
        None => find_manifest(&std::env::current_dir().map_err(|error| error.to_string())?)?,
    };
    let manifest_path = manifest_path
        .canonicalize()
        .map_err(|error| format!("cannot resolve `{}`: {error}", manifest_path.display()))?;
    let root = manifest_path
        .parent()
        .ok_or_else(|| "manifest does not have a parent directory".to_owned())?
        .to_owned();
    let manifest_source = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("cannot read `{}`: {error}", manifest_path.display()))?;
    let manifest: Manifest = toml::from_str(&manifest_source)
        .map_err(|error| format!("invalid `{}`: {error}", manifest_path.display()))?;
    validate_package_name(&manifest.package.name)?;
    if manifest.package.version.trim().is_empty() {
        return Err("package version cannot be empty".into());
    }
    let config_path = root.join(".slopium/config.toml");
    let config = if config_path.exists() {
        let source = fs::read_to_string(&config_path)
            .map_err(|error| format!("cannot read `{}`: {error}", config_path.display()))?;
        toml::from_str(&source)
            .map_err(|error| format!("invalid `{}`: {error}", config_path.display()))?
    } else {
        LocalConfig::default()
    };
    Ok(Project {
        root,
        manifest_path,
        manifest_source,
        manifest,
        config,
    })
}

fn find_manifest(start: &Path) -> Result<PathBuf, String> {
    for directory in start.ancestors() {
        let candidate = directory.join("Slopium.toml");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err("could not find `Slopium.toml` in this directory or its parents".into())
}

fn check(
    project: &Project,
    target_override: Option<String>,
    cc_override: Option<String>,
) -> Result<(), String> {
    let target = target(project, target_override);
    let source = source_path(project)?;
    let source_root = source_root(project)?;
    let dependencies = resolve_dependencies(project)?;
    let mut command = slopic_command(project, &target, cc_override)?;
    command.arg(&source).arg("--source-root").arg(&source_root);
    add_dependency_args(&mut command, &dependencies);
    let status = command
        .args(["--emit", "check", "--target", &target, "--profile", "dev"])
        .status()
        .map_err(|error| format!("cannot start slopic: {error}"))?;
    status_result(status, "check")?;
    println!(
        "Checked {} v{}",
        project.manifest.package.name, project.manifest.package.version
    );
    Ok(())
}

fn build(project: &Project, args: &BuildArgs, test: bool) -> Result<PathBuf, String> {
    let target = target(project, args.target.clone());
    if target != SUPPORTED_TARGET {
        return Err(format!(
            "target `{target}` is not installed; available target: `{SUPPORTED_TARGET}`"
        ));
    }
    let source = source_path(project)?;
    let source_root = source_root(project)?;
    let dependencies = resolve_dependencies(project)?;
    let profile_name = if args.release { "release" } else { "dev" };
    let profile = project.manifest.profile.get(profile_name);
    let out_dir = project.root.join("target").join(&target).join(profile_name);
    fs::create_dir_all(&out_dir)
        .map_err(|error| format!("cannot create `{}`: {error}", out_dir.display()))?;
    let artifact_name = if test {
        format!("{}-tests", project.manifest.package.name)
    } else {
        project.manifest.package.name.clone()
    };
    let artifact = out_dir.join(artifact_name);
    let stamp = artifact.with_extension("slop-cache");
    let compiler = slopic_path()?;
    verify_compiler(&compiler, &target)?;
    let runtime = materialize_runtime(&out_dir)?;
    let cc = cc_for(project, &target, args.cc.clone());
    let cache_inputs = CacheInputs {
        project,
        source_root: &source_root,
        dependencies: &dependencies,
        target: &target,
        profile_name,
        profile,
        test,
        compiler: &compiler,
        runtime: &runtime,
        cc: &cc,
    };
    let cache_key = cache_key(cache_inputs)?;
    if artifact.is_file() && fs::read_to_string(&stamp).ok().as_deref() == Some(&cache_key) {
        println!("Fresh {} ({profile_name})", project.manifest.package.name);
        return Ok(artifact);
    }

    println!(
        "Compiling {} v{} ({profile_name})",
        project.manifest.package.name, project.manifest.package.version
    );
    let object_dir = out_dir.join(if test { "test-objects" } else { "objects" });
    fs::create_dir_all(&object_dir)
        .map_err(|error| format!("cannot create `{}`: {error}", object_dir.display()))?;
    let mut objects = Vec::new();
    let module_units = codegen_module_units(project, &dependencies)?;
    for module in &module_units {
        let object = object_dir.join(format!("{}.o", encode_file_name(&module.name)));
        let object_stamp = object.with_extension("slop-cache");
        let module_key = module_cache_key(cache_inputs, module, &module_units)?;
        if !object.is_file()
            || fs::read_to_string(&object_stamp).ok().as_deref() != Some(&module_key)
        {
            let mut command = Command::new(&compiler);
            command.arg(&source).arg("--source-root").arg(&source_root);
            add_dependency_args(&mut command, &dependencies);
            command
                .args([
                    "--emit",
                    "obj",
                    "--target",
                    &target,
                    "--cc",
                    &cc,
                    "--profile",
                    profile_name,
                    "--codegen-module",
                    &module.name,
                ])
                .arg("--output")
                .arg(&object);
            if test {
                command.arg("--test");
            }
            if debug_info(profile, profile_name) {
                command.arg("--debug");
            }
            let status = command
                .status()
                .map_err(|error| format!("cannot start slopic: {error}"))?;
            status_result(status, &format!("codegen for module `{}`", module.name))?;
            fs::write(&object_stamp, module_key)
                .map_err(|error| format!("cannot write module cache stamp: {error}"))?;
        }
        objects.push(object);
    }
    let status = Command::new(&cc)
        .arg("-o")
        .arg(&artifact)
        .args(&objects)
        .arg(&runtime)
        .status()
        .map_err(|error| format!("cannot link package with `{cc}`: {error}"))?;
    status_result(status, "link")?;
    fs::write(&stamp, &cache_key)
        .map_err(|error| format!("cannot write build cache stamp: {error}"))?;
    println!("Finished {} ({})", profile_name, artifact.display());
    Ok(artifact)
}

#[derive(Clone, Debug)]
struct ModuleCacheUnit {
    name: String,
    source: String,
    interface: String,
    has_generics: bool,
}

fn codegen_module_units(
    project: &Project,
    dependencies: &[ResolvedDependency],
) -> Result<Vec<ModuleCacheUnit>, String> {
    fn modules(
        root: &Path,
        namespace: Option<&str>,
        output: &mut Vec<ModuleCacheUnit>,
    ) -> Result<(), String> {
        let mut sources = Vec::new();
        collect_cache_sources(root, &mut sources)?;
        for source in sources {
            let relative = source.strip_prefix(root).map_err(|error| {
                format!(
                    "cannot map source `{}` relative to `{}`: {error}",
                    source.display(),
                    root.display()
                )
            })?;
            let mut parts = relative
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            let Some(last) = parts.last_mut() else {
                continue;
            };
            *last = Path::new(last)
                .file_stem()
                .and_then(|name| name.to_str())
                .ok_or_else(|| format!("invalid source file name `{}`", source.display()))?
                .to_owned();
            let module = parts.join(":");
            let name = namespace.map_or(module.clone(), |prefix| format!("{prefix}:{module}"));
            let text = fs::read_to_string(&source)
                .map_err(|error| format!("cannot read `{}`: {error}", source.display()))?;
            let (interface, has_generics) = module_interface(&source.display().to_string(), &text)?;
            output.push(ModuleCacheUnit {
                name,
                source: text,
                interface,
                has_generics,
            });
        }
        Ok(())
    }

    let mut output = Vec::new();
    modules(&source_root(project)?, None, &mut output)?;
    for dependency in dependencies {
        match &dependency.source {
            ResolvedDependencySource::Path(root) => {
                modules(root, Some(&dependency.namespace), &mut output)?;
            }
            ResolvedDependencySource::Toolchain => {
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
                    let name = format!("{}:{module}", dependency.namespace);
                    let (interface, has_generics) =
                        module_interface(&format!("<toolchain>/{name}.slp"), source)?;
                    output.push(ModuleCacheUnit {
                        name,
                        source: source.into(),
                        interface,
                        has_generics,
                    });
                }
            }
        }
    }
    output.sort_by(|left, right| left.name.cmp(&right.name));
    output.dedup_by(|left, right| left.name == right.name);
    Ok(output)
}

fn module_interface(file: &str, source: &str) -> Result<(String, bool), String> {
    let tokens = slopic_core::lexer::lex(file, source).map_err(|diagnostics| {
        diagnostics
            .into_iter()
            .map(|diagnostic| diagnostic.message)
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    let forms = slopic_core::parser::parse(file, &tokens).map_err(|diagnostics| {
        diagnostics
            .into_iter()
            .map(|diagnostic| diagnostic.message)
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    let program = slopic_core::ast::build_program(file, &forms).map_err(|diagnostics| {
        diagnostics
            .into_iter()
            .map(|diagnostic| diagnostic.message)
            .collect::<Vec<_>>()
            .join("; ")
    })?;
    let mut interface = String::new();
    for export in &program.exports {
        interface.push_str("export");
        for item in &export.items {
            interface.push('|');
            interface.push_str(&item.path);
            interface.push('=');
            interface.push_str(&item.alias);
        }
        interface.push('\n');
    }
    for function in &program.functions {
        interface.push_str("fn|");
        interface.push_str(&function.name);
        interface.push('|');
        interface.push_str(&function.type_params.join(","));
        for parameter in &function.params {
            interface.push('|');
            interface.push_str(&parameter.name);
            interface.push(':');
            interface.push_str(&parameter.ty.to_string());
        }
        interface.push_str("->");
        interface.push_str(&function.return_type.to_string());
        interface.push('\n');
    }
    for structure in &program.structs {
        interface.push_str("struct|");
        interface.push_str(&structure.name);
        interface.push('|');
        interface.push_str(&structure.type_params.join(","));
        for field in &structure.fields {
            interface.push('|');
            interface.push_str(&field.name);
            interface.push(':');
            interface.push_str(&field.ty.to_string());
        }
        interface.push('\n');
    }
    for enumeration in &program.enums {
        interface.push_str("enum|");
        interface.push_str(&enumeration.name);
        interface.push('|');
        interface.push_str(&enumeration.type_params.join(","));
        for variant in &enumeration.variants {
            interface.push('|');
            interface.push_str(&variant.name);
            for field in &variant.fields {
                interface.push(':');
                interface.push_str(&field.name);
                interface.push('=');
                interface.push_str(&field.ty.to_string());
            }
        }
        interface.push('\n');
    }
    for test in &program.tests {
        interface.push_str("test|");
        interface.push_str(&test.name);
        interface.push('\n');
    }
    let has_generics = program
        .functions
        .iter()
        .any(|item| !item.type_params.is_empty())
        || program
            .structs
            .iter()
            .any(|item| !item.type_params.is_empty())
        || program
            .enums
            .iter()
            .any(|item| !item.type_params.is_empty());
    Ok((interface, has_generics))
}

fn encode_file_name(name: &str) -> String {
    name.bytes()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn run_artifact(artifact: &Path, args: &[OsString]) -> Result<(), String> {
    let status = Command::new(artifact)
        .args(args)
        .status()
        .map_err(|error| format!("cannot execute `{}`: {error}", artifact.display()))?;
    status_result(status, "program")
}

fn clean(project: Project) -> Result<(), String> {
    let target = project.root.join("target");
    if target.exists() {
        fs::remove_dir_all(&target)
            .map_err(|error| format!("cannot remove `{}`: {error}", target.display()))?;
        println!("Removed {}", target.display());
    }
    Ok(())
}

fn compiler_info() -> Result<(), String> {
    let status = Command::new(slopic_path()?)
        .arg("--info")
        .status()
        .map_err(|error| format!("cannot start slopic: {error}"))?;
    status_result(status, "compiler query")
}

fn slopic_command(
    project: &Project,
    target: &str,
    cc_override: Option<String>,
) -> Result<Command, String> {
    let compiler = slopic_path()?;
    verify_compiler(&compiler, target)?;
    let mut command = Command::new(compiler);
    command.args(["--cc", &cc_for(project, target, cc_override)]);
    Ok(command)
}

fn slopic_path() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("SLOPIC") {
        return Ok(PathBuf::from(path));
    }
    let current = std::env::current_exe()
        .map_err(|error| format!("cannot locate current executable: {error}"))?;
    let sibling = current.with_file_name("slopic");
    if sibling.is_file() {
        Ok(sibling)
    } else {
        Err(format!(
            "cannot find compatible `slopic` next to `{}`; set SLOPIC to its path",
            current.display()
        ))
    }
}

fn materialize_runtime(out_dir: &Path) -> Result<PathBuf, String> {
    let runtime = out_dir.join("slop_rt.c");
    let current = fs::read(&runtime).ok();
    if current.as_deref() != Some(slopic_core::RUNTIME_SOURCE) {
        fs::write(&runtime, slopic_core::RUNTIME_SOURCE)
            .map_err(|error| format!("cannot materialize `{}`: {error}", runtime.display()))?;
    }
    Ok(runtime)
}

fn source_path(project: &Project) -> Result<PathBuf, String> {
    let source = project.root.join(&project.manifest.package.entry);
    if !source.is_file() {
        return Err(format!(
            "entry source `{}` does not exist",
            source.display()
        ));
    }
    Ok(source)
}

fn source_root(project: &Project) -> Result<PathBuf, String> {
    let root = project
        .manifest
        .package
        .source
        .as_ref()
        .map(|source| project.root.join(source))
        .unwrap_or_else(|| {
            project
                .root
                .join(&project.manifest.package.entry)
                .parent()
                .unwrap_or(&project.root)
                .to_owned()
        });
    if !root.is_dir() {
        return Err(format!("source root `{}` does not exist", root.display()));
    }
    root.canonicalize()
        .map_err(|error| format!("cannot resolve source root `{}`: {error}", root.display()))
}

fn source_files(project: &Project) -> Result<Vec<PathBuf>, String> {
    let root = source_root(project)?;
    let mut files = Vec::new();
    collect_cache_sources(&root, &mut files)?;
    files.sort();
    Ok(files)
}

#[derive(Clone, Debug)]
enum ResolvedDependencySource {
    Path(PathBuf),
    Toolchain,
}

#[derive(Clone, Debug)]
struct ResolvedDependency {
    namespace: String,
    source: ResolvedDependencySource,
    language_items: Vec<(String, String)>,
    manifest_source: Option<String>,
}

fn resolve_dependencies(project: &Project) -> Result<Vec<ResolvedDependency>, String> {
    fn visit(
        project: &Project,
        prefix: Option<&str>,
        stack: &mut Vec<PathBuf>,
        seen: &mut HashSet<PathBuf>,
        output: &mut Vec<ResolvedDependency>,
    ) -> Result<(), String> {
        if stack.contains(&project.manifest_path) {
            let mut cycle = stack
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>();
            cycle.push(project.manifest_path.display().to_string());
            return Err(format!("package dependency cycle: {}", cycle.join(" -> ")));
        }
        stack.push(project.manifest_path.clone());
        let mut entries = project.manifest.dependencies.iter().collect::<Vec<_>>();
        entries.sort_by(|left, right| left.0.cmp(right.0));
        for (alias, specification) in entries {
            validate_package_name(alias)?;
            let namespace =
                prefix.map_or_else(|| alias.clone(), |prefix| format!("{prefix}:{alias}"));
            match specification {
                DependencySpec::Toolchain { toolchain } => {
                    if !*toolchain {
                        return Err(format!(
                            "dependency `{namespace}` has `toolchain = false`; use a path instead"
                        ));
                    }
                    if alias != "std" {
                        return Err(format!(
                            "dependency `{namespace}` cannot use the toolchain source; only `std` is bundled"
                        ));
                    }
                    output.push(ResolvedDependency {
                        namespace,
                        source: ResolvedDependencySource::Toolchain,
                        language_items: if prefix.is_none() {
                            vec![
                                ("option".into(), "std:option:Option".into()),
                                ("result".into(), "std:result:Result".into()),
                                ("result-ok".into(), "std:result:Ok".into()),
                                ("result-err".into(), "std:result:Err".into()),
                            ]
                        } else {
                            Vec::new()
                        },
                        manifest_source: None,
                    });
                }
                DependencySpec::Path { path } => {
                    let root = project.root.join(path);
                    let manifest = if root.is_dir() {
                        root.join("Slopium.toml")
                    } else {
                        root
                    };
                    let dependency = load_project(Some(manifest))?;
                    let source = source_root(&dependency)?;
                    if seen.insert(dependency.manifest_path.clone()) {
                        output.push(ResolvedDependency {
                            namespace: namespace.clone(),
                            source: ResolvedDependencySource::Path(source),
                            language_items: if prefix.is_none() && alias == "std" {
                                dependency
                                    .manifest
                                    .language_items
                                    .entries()
                                    .into_iter()
                                    .map(|(name, path)| (name, format!("{namespace}:{path}")))
                                    .collect()
                            } else {
                                Vec::new()
                            },
                            manifest_source: Some(dependency.manifest_source.clone()),
                        });
                    }
                    visit(&dependency, Some(&namespace), stack, seen, output)?;
                }
            }
        }
        stack.pop();
        Ok(())
    }

    let source_root = source_root(project)?;
    let mut local_roots = HashSet::new();
    let mut sources = Vec::new();
    collect_cache_sources(&source_root, &mut sources)?;
    for source in sources {
        let relative = source.strip_prefix(&source_root).map_err(|error| {
            format!(
                "cannot map source `{}` relative to `{}`: {error}",
                source.display(),
                source_root.display()
            )
        })?;
        let first = relative
            .components()
            .next()
            .and_then(|component| {
                let path = Path::new(component.as_os_str());
                path.file_stem().and_then(|name| name.to_str())
            })
            .ok_or_else(|| format!("invalid source path `{}`", source.display()))?;
        local_roots.insert(first.to_owned());
    }
    for alias in project.manifest.dependencies.keys() {
        if local_roots.contains(alias) {
            return Err(format!(
                "dependency alias `{alias}` collides with the local module namespace"
            ));
        }
    }

    let mut output = Vec::new();
    visit(
        project,
        None,
        &mut Vec::new(),
        &mut HashSet::new(),
        &mut output,
    )?;
    output.sort_by(|left, right| left.namespace.cmp(&right.namespace));
    Ok(output)
}

fn add_dependency_args(command: &mut Command, dependencies: &[ResolvedDependency]) {
    for dependency in dependencies {
        match &dependency.source {
            ResolvedDependencySource::Path(path) => {
                command.arg("--dependency").arg(format!(
                    "{}={}",
                    dependency.namespace,
                    path.display()
                ));
            }
            ResolvedDependencySource::Toolchain => {
                command
                    .arg("--toolchain-dependency")
                    .arg(&dependency.namespace);
            }
        }
        for (name, path) in &dependency.language_items {
            command.arg("--language-item").arg(format!("{name}={path}"));
        }
    }
}

fn target(project: &Project, override_target: Option<String>) -> String {
    override_target
        .or_else(|| std::env::var("SLOPIUM_TARGET").ok())
        .or_else(|| project.manifest.build.target.clone())
        .unwrap_or_else(|| SUPPORTED_TARGET.into())
}

/// Whether a profile emits DWARF line tables.
///
/// An absent `debug` means the conventional default: on for `dev`, off for
/// `release`. The build caches hash this resolved answer rather than the raw
/// field, because an absent field and an explicit `debug = false` hash alike
/// while resolving differently under `dev`.
fn debug_info(profile: Option<&Profile>, profile_name: &str) -> bool {
    profile
        .and_then(|profile| profile.debug)
        .unwrap_or(profile_name == "dev")
}

fn cc_for(project: &Project, target: &str, override_cc: Option<String>) -> String {
    let normalized = target.replace('-', "_").to_ascii_uppercase();
    override_cc
        .or_else(|| std::env::var(format!("SLOPIUM_CC_{normalized}")).ok())
        .or_else(|| {
            project
                .config
                .target
                .get(target)
                .and_then(|config| config.cc.clone())
        })
        .or_else(|| project.config.toolchain.cc.clone())
        .unwrap_or_else(|| "cc".into())
}

#[derive(Clone, Copy)]
struct CacheInputs<'a> {
    project: &'a Project,
    source_root: &'a Path,
    dependencies: &'a [ResolvedDependency],
    target: &'a str,
    profile_name: &'a str,
    profile: Option<&'a Profile>,
    test: bool,
    compiler: &'a Path,
    runtime: &'a Path,
    cc: &'a str,
}

fn cache_key(input: CacheInputs<'_>) -> Result<String, String> {
    let mut hasher = Fnv1a::default();
    hasher.write(input.project.manifest_source.as_bytes());
    let mut sources = Vec::new();
    collect_cache_sources(input.source_root, &mut sources)?;
    sources.sort();
    for source in sources {
        hasher.write(source.display().to_string().as_bytes());
        hasher.write(
            &fs::read(&source)
                .map_err(|error| format!("cannot hash `{}`: {error}", source.display()))?,
        );
    }
    for dependency in input.dependencies {
        hasher.write(dependency.namespace.as_bytes());
        if let Some(manifest) = &dependency.manifest_source {
            hasher.write(manifest.as_bytes());
        }
        match &dependency.source {
            ResolvedDependencySource::Toolchain => {
                hasher.write(b"toolchain");
            }
            ResolvedDependencySource::Path(root) => {
                let mut sources = Vec::new();
                collect_cache_sources(root, &mut sources)?;
                sources.sort();
                for source in sources {
                    hasher.write(source.display().to_string().as_bytes());
                    hasher.write(&fs::read(&source).map_err(|error| {
                        format!("cannot hash dependency `{}`: {error}", source.display())
                    })?);
                }
            }
        }
    }
    hasher.write(input.target.as_bytes());
    hasher.write(input.profile_name.as_bytes());
    hasher.write(&[u8::from(input.test)]);
    if let Some(profile) = input.profile {
        hasher.write(&[profile.opt_level.unwrap_or_default()]);
    }
    hasher.write(&[u8::from(debug_info(input.profile, input.profile_name))]);
    let metadata = fs::metadata(input.compiler)
        .map_err(|error| format!("cannot inspect `{}`: {error}", input.compiler.display()))?;
    hasher.write(&metadata.len().to_le_bytes());
    if let Ok(modified) = metadata.modified() {
        if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
            hasher.write(&duration.as_nanos().to_le_bytes());
        }
    }
    hasher.write(
        &fs::read(input.runtime)
            .map_err(|error| format!("cannot hash `{}`: {error}", input.runtime.display()))?,
    );
    hasher.write(input.cc.as_bytes());
    Ok(format!("{:016x}", hasher.finish()))
}

fn module_cache_key(
    input: CacheInputs<'_>,
    unit: &ModuleCacheUnit,
    units: &[ModuleCacheUnit],
) -> Result<String, String> {
    let mut hasher = Fnv1a::default();
    hasher.write(b"slopium-object-cache-v2");
    hasher.write(input.project.manifest_source.as_bytes());
    hasher.write(input.target.as_bytes());
    hasher.write(input.profile_name.as_bytes());
    hasher.write(&[u8::from(input.test)]);
    if let Some(profile) = input.profile {
        hasher.write(&[profile.opt_level.unwrap_or_default()]);
    }
    hasher.write(&[u8::from(debug_info(input.profile, input.profile_name))]);
    let metadata = fs::metadata(input.compiler)
        .map_err(|error| format!("cannot inspect `{}`: {error}", input.compiler.display()))?;
    hasher.write(&metadata.len().to_le_bytes());
    if let Ok(modified) = metadata.modified() {
        if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
            hasher.write(&duration.as_nanos().to_le_bytes());
        }
    }
    hasher.write(input.cc.as_bytes());
    for dependency in input.dependencies {
        hasher.write(dependency.namespace.as_bytes());
        if let Some(manifest) = &dependency.manifest_source {
            hasher.write(manifest.as_bytes());
        }
        for (name, path) in &dependency.language_items {
            hasher.write(name.as_bytes());
            hasher.write(path.as_bytes());
        }
    }
    hasher.write(unit.name.as_bytes());
    hasher.write(unit.source.as_bytes());
    for candidate in units {
        hasher.write(candidate.name.as_bytes());
        hasher.write(candidate.interface.as_bytes());
        if unit.has_generics {
            hasher.write(candidate.source.as_bytes());
        }
    }
    Ok(format!("{:016x}", hasher.finish()))
}

fn collect_cache_sources(directory: &Path, output: &mut Vec<PathBuf>) -> Result<(), String> {
    slopic_core::collect_slp_sources(directory, output)
        .map_err(|error| format!("cannot read `{}`: {error}", directory.display()))
}

fn verify_compiler(path: &Path, target: &str) -> Result<(), String> {
    let output = Command::new(path)
        .arg("--info")
        .output()
        .map_err(|error| format!("cannot query `{}`: {error}", path.display()))?;
    if !output.status.success() {
        return Err(format!("`{}` failed its version handshake", path.display()));
    }
    let info: CompilerHandshake = serde_json::from_slice(&output.stdout).map_err(|error| {
        format!(
            "invalid compiler handshake from `{}`: {error}",
            path.display()
        )
    })?;
    if info.protocol != slopic_core::COMPILER_PROTOCOL {
        return Err(format!(
            "incompatible slopic protocol {}; slopium requires {}",
            info.protocol,
            slopic_core::COMPILER_PROTOCOL
        ));
    }
    if !info.targets.iter().any(|installed| installed == target) {
        return Err(format!("compiler does not support target `{target}`"));
    }
    Ok(())
}

/// Build-cache digest.
///
/// This is a freshness check, not a security boundary: a path dependency that
/// wanted to influence the build can simply put code in its own sources, which
/// are compiled into the artifact by design. Accidental collisions are what
/// matter here, and 64 bits is ample for that.
struct Fnv1a(u64);

impl Default for Fnv1a {
    fn default() -> Self {
        Self(0xcbf29ce484222325)
    }
}

impl Hasher for Fnv1a {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut hash = self.0;
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        self.0 = hash;
    }
}

fn status_result(status: ExitStatus, action: &str) -> Result<(), String> {
    if status.success() {
        Ok(())
    } else {
        Err(format!("{action} failed with {status}"))
    }
}

fn validate_package_name(name: &str) -> Result<(), String> {
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'));
    if valid {
        Ok(())
    } else {
        Err(format!(
            "invalid package name `{name}`; use ASCII letters, digits, `-`, or `_`"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_manifest() {
        let manifest: Manifest = toml::from_str(
            r#"
                [package]
                name = "hello"
                version = "0.1.0"
                entry = "src/main.slp"
            "#,
        )
        .unwrap();
        assert_eq!(manifest.package.name, "hello");
    }

    #[test]
    fn fnv_is_stable() {
        let mut left = Fnv1a::default();
        left.write(b"slopium");
        let mut right = Fnv1a::default();
        right.write(b"slopium");
        assert_eq!(left.finish(), right.finish());
    }

    #[test]
    fn formatter_check_does_not_write_and_format_is_atomic() {
        let root = std::env::temp_dir().join(format!("slopium-format-test-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        create_project("format-test", Some(root.clone())).unwrap();
        let source_path = root.join("src/main.slp");
        let unformatted = "(fn main () -> i32   ; keep\n 0)";
        fs::write(&source_path, unformatted).unwrap();
        let project = load_project(Some(root.join("Slopium.toml"))).unwrap();

        assert!(format_project(&project, true).is_err());
        assert_eq!(fs::read_to_string(&source_path).unwrap(), unformatted);
        format_project(&project, false).unwrap();
        assert!(format_project(&project, true).is_ok());
        let formatted = fs::read_to_string(&source_path).unwrap();
        assert!(formatted.contains("; keep"));
        assert!(formatted.ends_with('\n'));

        fs::write(&source_path, "(fn main").unwrap();
        assert!(format_project(&project, false).is_err());
        assert_eq!(fs::read_to_string(&source_path).unwrap(), "(fn main");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn module_cache_ignores_other_bodies_but_tracks_interfaces() {
        let root = std::env::temp_dir().join(format!("slopium-cache-test-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        create_project("cache-test", Some(root.clone())).unwrap();
        let project = load_project(Some(root.join("Slopium.toml"))).unwrap();
        let source_root = source_root(&project).unwrap();
        let compiler = root.join("Slopium.toml");
        let runtime = root.join("src/main.slp");
        let inputs = CacheInputs {
            project: &project,
            source_root: &source_root,
            dependencies: &[],
            target: SUPPORTED_TARGET,
            profile_name: "dev",
            profile: project.manifest.profile.get("dev"),
            test: false,
            compiler: &compiler,
            runtime: &runtime,
            cc: "cc",
        };
        let unit = |name: &str, source: &str| {
            let (interface, has_generics) = module_interface(name, source).unwrap();
            ModuleCacheUnit {
                name: name.into(),
                source: source.into(),
                interface,
                has_generics,
            }
        };
        let main = unit(
            "main",
            "(take helper answer)\n(fn main () -> i32 (do (println (answer)) 0))",
        );
        let helper = unit("helper", "(export answer)\n(fn answer () -> i64 42)");
        let original = vec![helper.clone(), main.clone()];
        let body_changed = vec![
            unit("helper", "(export answer)\n(fn answer () -> i64 43)"),
            main.clone(),
        ];
        assert_eq!(
            module_cache_key(inputs, &main, &original).unwrap(),
            module_cache_key(inputs, &main, &body_changed).unwrap()
        );
        assert_ne!(
            module_cache_key(inputs, &helper, &original).unwrap(),
            module_cache_key(inputs, &body_changed[0], &body_changed).unwrap()
        );

        let interface_changed = vec![
            unit("helper", "(export answer)\n(fn answer () -> i32 43)"),
            main.clone(),
        ];
        assert_ne!(
            module_cache_key(inputs, &main, &original).unwrap(),
            module_cache_key(inputs, &main, &interface_changed).unwrap()
        );
        fs::remove_dir_all(root).unwrap();
    }
}
