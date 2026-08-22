use clap::{Args, CommandFactory, Parser, Subcommand};
use clap_complete::{generate, Shell};
use serde::Deserialize;
use slopic_core::ast::{Annotation, AnnotationArg, Expr, ExprKind};
use slopic_core::codegen::{Environment, DEFAULT_TARGET, TARGETS};
use slopic_core::syntax::{format_source, FormatOptions};
use slopium_manifest::archive::{package_archive, ARCHIVE_EXTENSION};
use slopium_manifest::lock::{Lockfile, LOCK_FILE};
use slopium_manifest::manifest::{
    load_local_config, validate_package_name, LocalConfig, Profile, Project, MANIFEST_FILE,
};
use slopium_manifest::registry::{
    archive_path as published_archive_path, index_path as published_index_path,
    signature_path as published_signature_path, IndexDependency, IndexEntry, IndexSource,
    Registries, INDEX_DIRECTORY,
};
use slopium_manifest::resolve::{resolve_workspace, Resolution, WorkspaceResolution};
use slopium_manifest::sha256::Digest;
use slopium_manifest::signature::PrivateKey;
use slopium_manifest::source::{SourceId, SourceSpec, DEFAULT_REGISTRY};
use slopium_manifest::sources::Sources;
use slopium_manifest::std_library::{toolchain_module_path, toolchain_package};
use slopium_manifest::store::{remove_tree, Access, Store};
use slopium_manifest::version::Version;
use slopium_manifest::workspace::{enclosing_workspace, load_workspace, Enclosing, Workspace};
use std::collections::{BTreeMap, HashSet};
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
        /// Create a library package: entered through `src/lib.slp`, with no
        /// `main` and nothing to link.
        #[arg(long)]
        lib: bool,
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
        #[command(flatten)]
        select: SelectArgs,
    },
    Clean(SelectArgs),
    /// Write a package archive under `target/package` and print its digest.
    Package {
        /// Print the registry index line for the archive instead, and nothing
        /// else — which is what putting a package into a static index takes.
        #[arg(long)]
        index_entry: bool,
        #[command(flatten)]
        resolve: ResolveArgs,
        #[command(flatten)]
        select: SelectArgs,
    },
    /// Copy every dependency that is not a directory on this machine into a
    /// vendor directory, and redirect builds to read it from there.
    Vendor {
        /// Where the copies go, relative to the workspace root.
        #[arg(long, value_name = "DIR", default_value = "vendor")]
        dir: PathBuf,
        /// Copy only what this member needs, instead of everything the
        /// workspace resolves.
        ///
        /// The redirection this writes covers the whole workspace, so a member
        /// left out of the copy stops building `--offline`. Which ones those
        /// are is printed.
        #[arg(short, long, value_name = "NAME")]
        package: Option<String>,
        #[command(flatten)]
        resolve: ResolveArgs,
    },
    /// Add a dependency to `Slopium.toml` and resolve it.
    Add {
        /// `name`, or `name@<requirement>` for a registry dependency.
        spec: String,
        /// Take it from a repository instead of a registry.
        #[arg(long, value_name = "URL")]
        git: Option<String>,
        #[arg(long, value_name = "NAME", requires = "git")]
        branch: Option<String>,
        #[arg(long, value_name = "NAME", requires = "git")]
        tag: Option<String>,
        #[arg(long, value_name = "REV", requires = "git")]
        rev: Option<String>,
        /// Take it from a directory instead of a registry.
        #[arg(long, value_name = "DIR")]
        path: Option<PathBuf>,
        /// Take it from this configured registry instead of `default`.
        #[arg(long, value_name = "NAME")]
        registry: Option<String>,
        #[command(flatten)]
        resolve: ResolveArgs,
        #[command(flatten)]
        select: SelectArgs,
    },
    /// Remove a dependency from `Slopium.toml` and resolve again.
    Remove {
        name: String,
        #[command(flatten)]
        resolve: ResolveArgs,
        #[command(flatten)]
        select: SelectArgs,
    },
    /// Move packages the lockfile pins to whatever their source offers now.
    Update {
        /// Update only this package. Repeatable; without any, everything moves.
        #[arg(short, long, value_name = "NAME")]
        package: Vec<String>,
        /// Move the one named package to exactly this version.
        #[arg(long, value_name = "VERSION", requires = "package")]
        precise: Option<String>,
        #[command(flatten)]
        resolve: ResolveArgs,
    },
    /// Sign a package archive and write it into a registry.
    Publish {
        /// The signing key, as `slopium key new` writes one. Key material is
        /// never an argument: `/proc/<pid>/cmdline` is world-readable.
        #[arg(long, value_name = "FILE")]
        key: PathBuf,
        /// Publish to this configured registry instead of `default`.
        #[arg(long, value_name = "NAME")]
        registry: Option<String>,
        /// Do everything except write into the registry, and say what would
        /// have been written.
        #[arg(long)]
        dry_run: bool,
        #[command(flatten)]
        resolve: ResolveArgs,
        #[command(flatten)]
        select: SelectArgs,
    },
    /// Re-check every dependency in the store against its checksum and, where
    /// a registry has trusted keys, its signature.
    ///
    /// There is one lockfile per workspace, so this acts on all of it.
    Verify {
        #[command(flatten)]
        resolve: ResolveArgs,
    },
    /// Make and inspect the keys packages are published under.
    Key {
        #[command(subcommand)]
        command: KeyCommands,
    },
    /// Print the resolved package graph.
    Tree {
        /// Stop after this many levels. The root is level 0, so `--depth 1` is
        /// the direct dependencies. A subtree that is cut off is marked `(...)`.
        #[arg(long, value_name = "N")]
        depth: Option<usize>,
        /// List the packages more than one package depends on, with their
        /// dependents, instead of the tree.
        ///
        /// This graph holds one version per name — two of them is an error
        /// (`D-036`) — so a duplicate here is a shared dependency, never a
        /// second copy the way it is in Cargo.
        #[arg(long)]
        duplicates: bool,
        #[command(flatten)]
        resolve: ResolveArgs,
        #[command(flatten)]
        select: SelectArgs,
    },
    Targets,
    Compiler,
    /// Print a shell completion script for `slopium` on stdout.
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Subcommand)]
enum KeyCommands {
    /// Write a new signing key, and print the public half to paste into
    /// `[registry.<name>] trusted-keys`.
    New {
        /// Where the key goes. An existing file is never overwritten.
        path: PathBuf,
    },
    /// Print the public half of a signing key.
    Public { path: PathBuf },
}

#[derive(Args, Clone)]
struct TargetArgs {
    #[arg(long)]
    target: Option<String>,
    #[arg(long)]
    cc: Option<String>,
    #[command(flatten)]
    resolve: ResolveArgs,
    #[command(flatten)]
    select: SelectArgs,
}

#[derive(Args, Clone)]
struct BuildArgs {
    #[arg(long)]
    target: Option<String>,
    #[arg(long)]
    release: bool,
    #[arg(long)]
    cc: Option<String>,
    #[command(flatten)]
    resolve: ResolveArgs,
    #[command(flatten)]
    select: SelectArgs,
}

/// Which packages of a workspace a command acts on.
#[derive(Args, Clone, Default)]
struct SelectArgs {
    /// Act on this package instead of the one the working directory is in.
    #[arg(short, long, value_name = "NAME")]
    package: Option<String>,
    /// Act on every member of the workspace.
    #[arg(long)]
    workspace: bool,
}

/// How resolution is allowed to behave.
#[derive(Args, Clone, Copy, Default)]
struct ResolveArgs {
    /// Fail instead of writing `Slopium.lock`.
    #[arg(long)]
    locked: bool,
    /// Never reach the network: build from the lock, the package store and any
    /// vendored copies, and fail naming what is missing.
    #[arg(long)]
    offline: bool,
    /// `--locked` and `--offline` together.
    #[arg(long)]
    frozen: bool,
}

impl SelectArgs {
    /// The one package a command that cannot act on several is aimed at.
    fn one<'a>(&self, workspace: &'a Workspace, action: &str) -> Result<&'a Project, String> {
        if self.workspace {
            return Err(format!(
                "SL1060: `{action}` acts on one package, but `--workspace` names every member"
            ));
        }
        workspace.select_one(self.package.as_deref(), action)
    }

    fn all<'a>(&self, workspace: &'a Workspace) -> Result<Vec<&'a Project>, String> {
        workspace.select(self.package.as_deref(), self.workspace)
    }
}

impl ResolveArgs {
    fn locked(self) -> bool {
        self.locked || self.frozen
    }

    fn offline(self) -> bool {
        self.offline || self.frozen
    }

    fn access(self) -> Access {
        Access::new(self.offline())
    }
}

#[derive(Deserialize)]
struct CompilerHandshake {
    protocol: u32,
    targets: Vec<String>,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::New { name, path, lib } => create_project(&name, path, lib),
        Commands::Check(args) => {
            Session::open(cli.manifest_path, args.resolve).and_then(|session| {
                for project in args.select.all(&session.workspace)? {
                    check(&session, project, &args)?;
                }
                Ok(())
            })
        }
        Commands::Build(args) => {
            Session::open(cli.manifest_path, args.resolve).and_then(|session| {
                for project in args.select.all(&session.workspace)? {
                    build(&session, project, &args, false)?;
                }
                Ok(())
            })
        }
        Commands::Run {
            build: args,
            args: program_args,
        } => Session::open(cli.manifest_path, args.resolve).and_then(|session| {
            let project = args.select.one(&session.workspace, "run")?;
            if project.is_library() {
                return Err(format!(
                    "`{}` is a library and has no executable to run",
                    project.name
                ));
            }
            let artifact = build(&session, project, &args, false)?
                .ok_or_else(|| format!("`{}` produced no executable", project.name))?;
            run_artifact(&artifact, &program_args)
        }),
        Commands::Test(args) => {
            Session::open(cli.manifest_path, args.resolve).and_then(|session| {
                for project in args.select.all(&session.workspace)? {
                    let Some(artifact) = build(&session, project, &args, true)? else {
                        continue;
                    };
                    let status = Command::new(&artifact)
                        .status()
                        .map_err(|error| format!("cannot execute tests: {error}"))?;
                    status_result(status, "tests")?;
                }
                Ok(())
            })
        }
        Commands::Fmt { check, select } => {
            open_workspace(cli.manifest_path).and_then(|workspace| {
                let mut differences = Vec::new();
                for project in select.all(&workspace)? {
                    differences.extend(format_project(project, check)?);
                }
                if differences.is_empty() {
                    return Ok(());
                }
                Err(format!(
                    "formatting differs: {}",
                    differences
                        .iter()
                        .map(|path| path.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })
        }
        Commands::Clean(select) => {
            open_workspace(cli.manifest_path).and_then(|workspace| clean(&workspace, &select))
        }
        Commands::Package {
            index_entry,
            resolve,
            select,
        } => Session::open(cli.manifest_path, resolve).and_then(|session| {
            for project in select.all(&session.workspace)? {
                package(&session.workspace, project, index_entry)?;
            }
            Ok(())
        }),
        Commands::Vendor {
            dir,
            package,
            resolve,
        } => Session::open_ignoring_replacements(cli.manifest_path, resolve)
            .and_then(|session| vendor(&session, &dir, package.as_deref())),
        Commands::Add {
            spec,
            git,
            branch,
            tag,
            rev,
            path,
            registry,
            resolve,
            select,
        } => add(
            cli.manifest_path,
            &spec,
            Added {
                git,
                branch,
                tag,
                rev,
                path,
                registry,
            },
            resolve,
            &select,
        ),
        Commands::Remove {
            name,
            resolve,
            select,
        } => remove(cli.manifest_path, &name, resolve, &select),
        Commands::Update {
            package,
            precise,
            resolve,
        } => update(cli.manifest_path, package, precise, resolve),
        Commands::Publish {
            key,
            registry,
            dry_run,
            resolve,
            select,
        } => Session::open(cli.manifest_path, resolve).and_then(|session| {
            let project = select.one(&session.workspace, "publish")?;
            publish(&session, project, &key, registry.as_deref(), dry_run)
        }),
        Commands::Verify { resolve } => {
            Session::open(cli.manifest_path, resolve).and_then(|session| verify(&session))
        }
        Commands::Key { command } => match command {
            KeyCommands::New { path } => new_key(&path),
            KeyCommands::Public { path } => {
                PrivateKey::read(&path).map(|key| println!("{}", key.public()))
            }
        },
        Commands::Tree {
            depth,
            duplicates,
            resolve,
            select,
        } => Session::open(cli.manifest_path, resolve).and_then(|session| {
            for project in select.all(&session.workspace)? {
                tree(&session, project, depth, duplicates)?;
            }
            Ok(())
        }),
        Commands::Targets => {
            for spec in TARGETS {
                let note = if spec.triple == DEFAULT_TARGET {
                    "installed, default"
                } else {
                    "installed"
                };
                println!("{} ({note})", spec.triple);
            }
            Ok(())
        }
        Commands::Compiler => compiler_info(),
        Commands::Completions { shell } => {
            let mut command = Cli::command();
            let name = command.get_name().to_string();
            generate(shell, &mut command, name, &mut std::io::stdout());
            Ok(())
        }
    };

    if let Err(error) = result {
        eprintln!("slopium: {error}");
        std::process::exit(1);
    }
}

/// The workspace this command acts on, with every manifest key the toolchain
/// does not know reported once (`D-128`).
///
/// The reporting is here rather than in the manager's library half, which
/// prints nothing at all, and it is the one place a command loads a workspace,
/// so a key is named once however many members were selected afterwards.
fn open_workspace(manifest_path: Option<PathBuf>) -> Result<Workspace, String> {
    let workspace = load_workspace(manifest_path)?;
    for (path, key) in &workspace.unknown_keys {
        eprintln!(
            "slopium: warning[SL1200]: `{}` sets `{key}`, which this toolchain does not know; it is ignored",
            path.display()
        );
    }
    Ok(workspace)
}

/// Format one package, returning the files that were not already formatted.
fn format_project(project: &Project, check: bool) -> Result<Vec<PathBuf>, String> {
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
        // Only a file that was actually rewritten is reported. Announcing
        // `Formatted` for an unchanged file — or for any file at all under
        // `--check`, which writes nothing — describes work that did not happen.
        if formatted != source {
            if check {
                differences.push(source_path);
            } else {
                atomic_write(&source_path, formatted.as_bytes())?;
                println!("Formatted {}", source_path.display());
            }
        }
    }
    Ok(differences)
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

fn create_project(name: &str, path: Option<PathBuf>, library: bool) -> Result<(), String> {
    validate_package_name(name)?;
    let root = path.unwrap_or_else(|| PathBuf::from(name));
    if root.exists() {
        return Err(format!("destination `{}` already exists", root.display()));
    }
    fs::create_dir_all(root.join("src"))
        .map_err(|error| format!("cannot create project: {error}"))?;
    // A library has no `main` to run and nothing to link, so a target triple
    // and a release profile would describe work it never does.
    let entry = if library {
        "src/lib.slp"
    } else {
        "src/main.slp"
    };
    let build = if library {
        String::new()
    } else {
        format!("[build]\ntarget = \"{DEFAULT_TARGET}\"\n\n")
    };
    let manifest = format!(
        "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nsource = \"src\"\nentry = \"{entry}\"\n\n\
         [dependencies]\nstd = {{ toolchain = true }}\n\n\
         {build}\
         [profile.dev]\nopt-level = 0\ndebug = true\nstrip = false\npanic = \"message\"\n\n\
         [profile.release]\nopt-level = 1\ndebug = false\nstrip = true\npanic = \"message\"\n"
    );
    let source = if library {
        "(export add)\n\n\
         (fn add ((left i64) (right i64)) -> i64\n  (+ left right))\n\n\
         (test \"addition\"\n  (= (add 20 22) 42))\n"
            .to_owned()
    } else {
        format!(
            "(take std:io println)\n\n\
             (fn main () -> i32\n  (let message \"hello from {name}\")\n  (println (& message))\n  0)\n\n\
             (test \"arithmetic\"\n  (= (+ 20 22) 42))\n"
        )
    };
    fs::write(root.join("Slopium.toml"), manifest)
        .map_err(|error| format!("cannot write manifest: {error}"))?;
    fs::write(root.join(entry), source).map_err(|error| format!("cannot write source: {error}"))?;
    fs::write(root.join(".gitignore"), "/target/\n/.slopium/\n")
        .map_err(|error| format!("cannot write .gitignore: {error}"))?;
    let kind = if library { "library" } else { "package" };
    println!("Created {kind} `{name}` at {}", root.display());
    enlist_in_workspace(&root)
}

/// Add a freshly created package to the workspace it landed inside.
///
/// Without this the package is unbuildable the moment it is created:
/// `load_workspace` refuses a package that sits in a workspace directory
/// without being listed, so every command run inside it fails until somebody
/// edits the root manifest by hand.
///
/// A failure here is reported without unwinding the new package. The files are
/// correct and the fix is one line in a manifest, so deleting somebody's new
/// package because its root manifest is written in a form this command does not
/// edit would be the worse outcome — the message says which line to write.
fn enlist_in_workspace(root: &Path) -> Result<(), String> {
    let workspace_root = match enclosing_workspace(root)? {
        Enclosing::Nothing | Enclosing::Member(_) => return Ok(()),
        Enclosing::Unlisted(workspace_root) => workspace_root,
    };
    let relative = root
        .canonicalize()
        .map_err(|error| format!("cannot read `{}`: {error}", root.display()))?
        .strip_prefix(&workspace_root)
        .map_err(|_| {
            format!(
                "`{}` is not inside the workspace at `{}`",
                root.display(),
                workspace_root.display()
            )
        })?
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    let manifest_path = workspace_root.join(MANIFEST_FILE);
    let source = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("cannot read `{}`: {error}", manifest_path.display()))?;
    let edited = with_member(&source, &relative)?;
    fs::write(&manifest_path, edited)
        .map_err(|error| format!("cannot write `{}`: {error}", manifest_path.display()))?;
    println!(
        "Added `{relative}` to `[workspace] members` in {}",
        manifest_path.display()
    );
    Ok(())
}

/// One load of the workspace, one resolution of it, and one reconciliation with
/// the lock — shared by every command that compiles something.
///
/// Resolution covers the whole workspace even when a command acts on one
/// member, because the lock does: building `-p a` must not rewrite what `b`
/// recorded.
#[derive(Debug)]
struct Session {
    workspace: Workspace,
    resolution: WorkspaceResolution,
    /// Kept so that commands acting on the resolved graph — `vendor` — reach
    /// the store through the same policy resolution did.
    sources: Sources,
}

/// What `slopium update` asked to move.
///
/// A pin is what makes a build reproducible, so throwing one away is a command
/// of its own rather than something a build decides — and `-p` throwing away
/// exactly one is what lets the lock's diff prove what moved.
#[derive(Clone, Debug, Default)]
struct Update {
    /// Empty means every pin.
    packages: Vec<String>,
    precise: Option<Version>,
}

impl Session {
    fn open(manifest_path: Option<PathBuf>, args: ResolveArgs) -> Result<Self, String> {
        Self::load(manifest_path, args, true, None)
    }

    fn open_updating(
        manifest_path: Option<PathBuf>,
        args: ResolveArgs,
        update: &Update,
    ) -> Result<Self, String> {
        Self::load(manifest_path, args, true, Some(update))
    }

    /// The workspace resolved as if nothing had been vendored.
    ///
    /// `vendor` is what produces the vendored copies, so it cannot insist they
    /// are already present and intact before it will run: an edited copy would
    /// then be unrepairable by the only command able to repair it. Replacement
    /// changes no resolved package (`D-047`), so this reaches the same graph
    /// and writes the same lock — only the bytes it reads differ.
    fn open_ignoring_replacements(
        manifest_path: Option<PathBuf>,
        args: ResolveArgs,
    ) -> Result<Self, String> {
        Self::load(manifest_path, args, false, None)
    }

    fn load(
        manifest_path: Option<PathBuf>,
        args: ResolveArgs,
        replacements: bool,
        update: Option<&Update>,
    ) -> Result<Self, String> {
        let mut workspace = open_workspace(manifest_path)?;
        if !replacements {
            workspace.config.source.clear();
        }
        // The lock is read before resolution rather than after it: what it
        // pinned is an input to resolving a source that moves, and reading it
        // twice would mean reporting an unreadable one twice.
        let existing = read_lock(&workspace, args)?;
        let mut sources = Sources::new(Store::open()?, args.access(), args.locked())
            .with_lock(existing.as_ref())
            .with_registries(Registries::from_config(&workspace.config, &workspace.root)?);
        if let Some(update) = update {
            let known = sources.pinned_names().join(", ");
            for name in &update.packages {
                if sources.pinned(name).is_none() {
                    return Err(format!(
                        "`{}` pins no `{name}`, so there is nothing to update; it holds {known}",
                        LOCK_FILE
                    ));
                }
            }
            sources = match update.packages.as_slice() {
                [] => sources.updating_everything(),
                names => sources.updating(names.iter().cloned()),
            };
            if let (Some(version), [name]) = (&update.precise, update.packages.as_slice()) {
                sources = sources.at_precisely(name, version.clone());
            }
        }
        let toolchain_version = Version::parse(slopic_core::STANDARD_LIBRARY_VERSION)?;
        let resolution = resolve_workspace(&workspace, &toolchain_version, &sources)?;
        synchronize_lock(&workspace, &resolution, existing, args)?;
        Ok(Self {
            workspace,
            resolution,
            sources,
        })
    }

    /// What one member compiles against.
    fn dependencies(&self, project: &Project, target: &str) -> Result<Dependencies, String> {
        let resolution = self.resolution.member(&project.name)?;
        dependencies_of(project, resolution, target)
    }
}

fn check(session: &Session, project: &Project, args: &TargetArgs) -> Result<(), String> {
    let target = target(project, args.target.clone());
    let source = source_path(project)?;
    let source_root = project.source_root()?;
    let dependencies = session.dependencies(project, &target)?;
    let selection = target_selection(project, &target)?;
    let mut command = slopic_command(project, &target, args.cc.clone())?;
    command.arg(&source).arg("--source-root").arg(&source_root);
    add_selection_args(&mut command, &selection);
    add_dependency_args(&mut command, &dependencies);
    if project.is_library() {
        command.arg("--library");
    }
    let status = command
        .args(["--emit", "check", "--target", &target])
        .status()
        .map_err(|error| format!("cannot start slopic: {error}"))?;
    status_result(status, "check")?;
    println!("Checked {} v{}", project.name, project.version);
    Ok(())
}

/// Build one package, or check it when there is nothing to link.
///
/// A library has no `main`, so `build` on one means "compile it and say so":
/// erroring would make `build --workspace` useless in any workspace that holds
/// a library, and linking would fail on the missing entry point. `test` still
/// produces an executable, because the harness supplies its own entry point.
fn build(
    session: &Session,
    project: &Project,
    args: &BuildArgs,
    test: bool,
) -> Result<Option<PathBuf>, String> {
    if project.is_library() && !test {
        check(
            session,
            project,
            &TargetArgs {
                target: args.target.clone(),
                cc: args.cc.clone(),
                resolve: args.resolve,
                select: SelectArgs::default(),
            },
        )?;
        return Ok(None);
    }
    let target = target(project, args.target.clone());
    if !TARGETS.iter().any(|spec| spec.triple == target) {
        let available: Vec<&str> = TARGETS.iter().map(|spec| spec.triple).collect();
        return Err(format!(
            "target `{target}` is not installed; available targets: {}",
            available.join(", ")
        ));
    }
    // The target says which environment this is, so the manager needs no
    // manifest boolean and no `--freestanding` of its own (`D-081`).
    let environment = slopic_core::environment_for(&target);
    // A freestanding target has no test harness and cannot be given one: the
    // `main` the compiler would generate calls `sl_rt_args_init` and
    // `sl_rt_test_result`, and both are defined only in the hosted half of the
    // runtime. Without this the harness is simply suppressed, and the binary
    // that comes out runs no test and says nothing about it.
    if test && environment == Environment::Freestanding {
        return Err(format!(
            "target `{target}` is freestanding, so it has no test harness; \
             a test needs a hosted target"
        ));
    }
    let source = source_path(project)?;
    let source_root = project.source_root()?;
    let dependencies = session.dependencies(project, &target)?;
    let selection = target_selection(project, &target)?;
    let profile_name = if args.release { "release" } else { "dev" };
    let profile = project.manifest.profile.get(profile_name);
    // One `target/` for the whole workspace, so members share compiled
    // dependencies instead of each rebuilding them under their own root.
    let out_dir = session
        .workspace
        .target_dir()
        .join(&target)
        .join(profile_name);
    fs::create_dir_all(&out_dir)
        .map_err(|error| format!("cannot create `{}`: {error}", out_dir.display()))?;
    let artifact_name = if test {
        format!("{}-tests", project.name)
    } else {
        project.name.clone()
    };
    let artifact = out_dir.join(artifact_name);
    let stamp = artifact.with_extension("slop-cache");
    let compiler = slopic_path()?;
    verify_compiler(&compiler, &target)?;
    let runtimes = materialize_runtime(&out_dir, environment)?;
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
        runtimes: &runtimes,
        cc: &cc,
    };
    let cache_key = cache_key(cache_inputs)?;
    if artifact.is_file() && fs::read_to_string(&stamp).ok().as_deref() == Some(&cache_key) {
        println!("Fresh {} ({profile_name})", project.name);
        return Ok(Some(artifact));
    }

    println!(
        "Compiling {} v{} ({profile_name})",
        project.name, project.version
    );
    // Per package, because two members compile a module of the same name into
    // objects with the same encoded file name.
    let object_dir = out_dir
        .join(if test { "test-objects" } else { "objects" })
        .join(&project.name);
    fs::create_dir_all(&object_dir)
        .map_err(|error| format!("cannot create `{}`: {error}", object_dir.display()))?;
    let mut objects = Vec::new();
    let module_units = codegen_module_units(project, &dependencies, &selection)?;
    for module in &module_units {
        let object = object_dir.join(format!("{}.o", encode_file_name(&module.name)));
        let object_stamp = object.with_extension("slop-cache");
        let module_key = module_cache_key(cache_inputs, module, &module_units)?;
        if !object.is_file()
            || fs::read_to_string(&object_stamp).ok().as_deref() != Some(&module_key)
        {
            let mut command = Command::new(&compiler);
            command.arg(&source).arg("--source-root").arg(&source_root);
            add_selection_args(&mut command, &selection);
            add_dependency_args(&mut command, &dependencies);
            command
                .args([
                    "--emit",
                    "obj",
                    "--target",
                    &target,
                    "--cc",
                    &cc,
                    "--codegen-module",
                    &module.name,
                ])
                .arg("--output")
                .arg(&object);
            if test {
                command.arg("--test");
            }
            if optimizes(profile, profile_name) {
                command.arg("--optimize");
            }
            if debug_info(profile, profile_name) {
                command.arg("--debug");
            }
            if panic_abort(profile) {
                command.arg("--panic-abort");
            }
            let status = command
                .status()
                .map_err(|error| format!("cannot start slopic: {error}"))?;
            codegen_status_result(status, &module.name)?;
            fs::write(&object_stamp, module_key)
                .map_err(|error| format!("cannot write module cache stamp: {error}"))?;
        }
        objects.push(object);
    }
    // The C an `extern` names, compiled with the same `cc` the link uses so
    // there is no second toolchain to configure or disagree with (`D-075`).
    // These are not cached per file: they are already inputs to `cache_key`, so
    // reaching here at all means one of them may have changed.
    let c_sources: Vec<PathBuf> = c_source_paths(project)
        .into_iter()
        .chain(dependencies.c_sources.iter().cloned())
        .collect();
    for (index, c_source) in c_sources.iter().enumerate() {
        if !c_source.is_file() {
            return Err(format!(
                "`c-sources` names `{}`, which is not a file",
                c_source.display()
            ));
        }
        // Indexed, because two packages may each carry a `hal.c`.
        let name = c_source.file_name().unwrap_or_default().to_string_lossy();
        let object = object_dir.join(format!("c-{index}-{}.o", encode_file_name(&name)));
        let status = Command::new(&cc)
            .arg("-c")
            .arg(c_source)
            .arg("-o")
            .arg(&object)
            .args(slopic_core::cc_compile_flags(environment))
            .status()
            .map_err(|error| {
                format!(
                    "cannot compile `{}` with `{cc}`: {error}",
                    c_source.display()
                )
            })?;
        status_result(status, &format!("compile of `{}`", c_source.display()))?;
        objects.push(object);
    }
    let mut link = Command::new(&cc);
    link.arg("-o")
        .arg(&artifact)
        // The same size and environment flags `slopic` uses for a single-file
        // link, so a package binary and a standalone one are shrunk, stripped
        // and hosted or freestanding alike.
        .args(slopic_core::cc_flags(
            environment,
            strip_symbols(profile, profile_name),
            panic_abort(profile),
        ));
    // Only the root package's script is consulted, because `[build]` is the
    // root's table (`D-117`). It is passed after the flags and before the
    // objects, where a `cc` driver expects it.
    if let Some(script) = linker_script_path(project) {
        if !script.is_file() {
            return Err(format!(
                "`linker-script` names `{}`, which is not a file",
                script.display()
            ));
        }
        link.arg("-T").arg(&script);
    }
    let status = link
        .args(&objects)
        .args(&runtimes)
        .status()
        .map_err(|error| format!("cannot link package with `{cc}`: {error}"))?;
    status_result(status, "link")?;
    fs::write(&stamp, &cache_key)
        .map_err(|error| format!("cannot write build cache stamp: {error}"))?;
    println!("Finished {} ({})", profile_name, artifact.display());
    Ok(Some(artifact))
}

#[derive(Clone, Debug)]
struct ModuleCacheUnit {
    name: String,
    source: String,
    interface: String,
    has_generics: bool,
}

/// One `ModuleCacheUnit` per module the build emits an object for.
///
/// It walks the source trees itself rather than asking the compiler, so it has
/// to leave out the same modules the compiler was told to (`D-135`): asking for
/// an object for a module that is not in the build would name a `--codegen-
/// module` the compiler has never heard of.
fn codegen_module_units(
    project: &Project,
    dependencies: &Dependencies,
    selection: &TargetSelection,
) -> Result<Vec<ModuleCacheUnit>, String> {
    fn modules(
        root: &Path,
        namespace: Option<&str>,
        excluded: &[String],
        named: &[(String, PathBuf)],
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
            // A file the manifest named is that module, whatever its path
            // would have said, which is exactly what the compiler was told.
            let module = named
                .iter()
                .find(|(_, path)| *path == source)
                .map_or_else(|| parts.join(":"), |(name, _)| name.clone());
            // Against the unqualified name, which is what the compiler was
            // handed: an exclusion belongs to the root it was read from.
            if excluded.contains(&module) {
                continue;
            }
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
    modules(
        &project.source_root()?,
        None,
        &selection.excluded,
        &selection.named,
        &mut output,
    )?;
    for dependency in &dependencies.packages {
        match &dependency.source {
            ResolvedDependencySource::Path(root) => {
                modules(
                    root,
                    Some(&dependency.namespace),
                    &dependency.excluded_modules,
                    &dependency.named_modules,
                    &mut output,
                )?;
            }
            // A toolchain dependency's alias is its package name — `SL1077`
            // refuses any other — so the namespace names the bundled package.
            ResolvedDependencySource::Toolchain => {
                let Some(bundled) = toolchain_package(&dependency.namespace) else {
                    continue;
                };
                for (module, source) in bundled.modules {
                    let name = format!("{}:{module}", dependency.namespace);
                    let (interface, has_generics) =
                        module_interface(&toolchain_module_path(bundled.name, module), source)?;
                    output.push(ModuleCacheUnit {
                        name,
                        source: (*source).into(),
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
    let tokens = slopic_core::reader::expand(file, &tokens).map_err(|diagnostics| {
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
        interface.push_str(&interface_annotations(&function.annotations));
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
    // An extern is callable from another module, so its signature is interface
    // in exactly the way a `fn`'s is — and the C symbol belongs here too, since
    // changing it changes what every caller's object asks the linker for.
    for declaration in &program.externs {
        interface.push_str("extern|");
        interface.push_str(&declaration.name);
        interface.push_str(&interface_annotations(&declaration.annotations));
        interface.push('|');
        interface.push_str(&declaration.symbol);
        for parameter in &declaration.params {
            interface.push('|');
            interface.push_str(&parameter.name);
            interface.push(':');
            interface.push_str(&parameter.ty.to_string());
        }
        interface.push_str("->");
        interface.push_str(&declaration.return_type.to_string());
        interface.push('\n');
    }
    for structure in &program.structs {
        interface.push_str("struct|");
        interface.push_str(&structure.name);
        interface.push_str(&interface_annotations(&structure.annotations));
        interface.push('|');
        interface.push_str(&structure.type_params.join(","));
        for field in &structure.fields {
            interface.push('|');
            interface.push_str(&field.name);
            // A field's `deprecated` is interface for the reason every
            // `deprecated` is: what warns is the module that names the field,
            // so a dependent that does not rebuild is a warning nobody sees
            // (`D-152`).
            interface.push_str(&interface_annotations(&field.annotations));
            interface.push(':');
            interface.push_str(&field.ty.to_string());
        }
        interface.push('\n');
    }
    for enumeration in &program.enums {
        interface.push_str("enum|");
        interface.push_str(&enumeration.name);
        interface.push_str(&interface_annotations(&enumeration.annotations));
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
    // A `const` is inlined wherever it is used (`D-121`), so its value is
    // interface in the strongest sense there is: a dependent that does not
    // rebuild keeps the old number compiled into it. Until this line the
    // interface did not mention constants at all.
    for constant in &program.consts {
        interface.push_str("const|");
        interface.push_str(&constant.name);
        interface.push_str(&interface_annotations(&constant.annotations));
        interface.push('|');
        interface.push_str(&const_value(&constant.value));
        if let Some(ty) = &constant.ty {
            interface.push(':');
            interface.push_str(&ty.to_string());
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

/// The annotations of a declaration that belong in its module's interface.
///
/// Which ones those are is `slopic_core::ast`'s table to say, not a second
/// list here that could disagree with it: an annotation a *caller* can observe
/// — `deprecated`, whose warning is raised at the call — has to rebuild what
/// depends on the module, and one that only changes this module's own object —
/// `inline` — must not.
fn interface_annotations(annotations: &[Annotation]) -> String {
    let mut rendered = String::new();
    for annotation in annotations
        .iter()
        .filter(|annotation| annotation.is_interface())
    {
        rendered.push_str("|@");
        rendered.push_str(&annotation.name);
        for argument in &annotation.args {
            rendered.push('=');
            match argument {
                AnnotationArg::Text(text) => rendered.push_str(text),
                AnnotationArg::Name(name) => rendered.push_str(name),
            }
        }
    }
    rendered
}

/// A `const`'s literal, rendered for the interface it is part of.
///
/// A hexadecimal literal is not the same as the decimal one of equal magnitude
/// — `D-112` makes it a bit pattern rather than a number — so the base is part
/// of the value here.
fn const_value(value: &Expr) -> String {
    match &value.kind {
        ExprKind::Unit => "unit".to_owned(),
        ExprKind::Bool(value) => value.to_string(),
        ExprKind::Int(literal) => format!(
            "{}{}{}",
            if literal.negative { "-" } else { "" },
            literal.magnitude,
            if literal.bits { "b" } else { "" }
        ),
        ExprKind::Float(value) => format!("{value:?}"),
        ExprKind::String(bytes) => String::from_utf8_lossy(bytes).into_owned(),
        // `ast` refuses a `const` that is not a literal, so reaching here means
        // a diagnostic has already been reported about this declaration.
        _ => String::new(),
    }
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

/// Remove build output: the whole workspace's, or one package's.
///
/// Naming a package removes what that package produced under every target and
/// profile and leaves its dependencies' objects alone, which is the only reason
/// to name one at all.
fn clean(workspace: &Workspace, select: &SelectArgs) -> Result<(), String> {
    let target = workspace.target_dir();
    if !target.exists() {
        return Ok(());
    }
    if select.package.is_none() {
        fs::remove_dir_all(&target)
            .map_err(|error| format!("cannot remove `{}`: {error}", target.display()))?;
        println!("Removed {}", target.display());
        return Ok(());
    }
    for project in select.all(workspace)? {
        for profile_dir in profile_directories(&target)? {
            let artifacts = [
                profile_dir.join(&project.name),
                profile_dir.join(format!("{}.slop-cache", project.name)),
                profile_dir.join(format!("{}-tests", project.name)),
                profile_dir.join(format!("{}-tests.slop-cache", project.name)),
            ];
            for artifact in artifacts {
                if artifact.is_file() {
                    fs::remove_file(&artifact).map_err(|error| {
                        format!("cannot remove `{}`: {error}", artifact.display())
                    })?;
                }
            }
            for objects in ["objects", "test-objects"] {
                let directory = profile_dir.join(objects).join(&project.name);
                if directory.is_dir() {
                    fs::remove_dir_all(&directory).map_err(|error| {
                        format!("cannot remove `{}`: {error}", directory.display())
                    })?;
                }
            }
        }
        println!("Removed build output for {}", project.name);
    }
    Ok(())
}

/// Every `<target>/<profile>` directory under `target/`.
fn profile_directories(target: &Path) -> Result<Vec<PathBuf>, String> {
    let mut directories = Vec::new();
    let triples = fs::read_dir(target)
        .map_err(|error| format!("cannot read `{}`: {error}", target.display()))?;
    for triple in triples {
        let triple =
            triple.map_err(|error| format!("cannot read `{}`: {error}", target.display()))?;
        if !triple.path().is_dir() {
            continue;
        }
        let profiles = fs::read_dir(triple.path())
            .map_err(|error| format!("cannot read `{}`: {error}", triple.path().display()))?;
        for profile in profiles {
            let profile = profile
                .map_err(|error| format!("cannot read `{}`: {error}", triple.path().display()))?;
            if profile.path().is_dir() {
                directories.push(profile.path());
            }
        }
    }
    Ok(directories)
}

/// Print the resolved graph.
///
/// A package appears once per place it is depended on, but is only expanded the
/// first time — the repeat is marked `(*)`. That is the visible shape of
/// `D-035`: a diamond shows the shared package twice and builds it once.
/// What `slopium add` was told to write.
#[derive(Debug, Default)]
struct Added {
    git: Option<String>,
    branch: Option<String>,
    tag: Option<String>,
    rev: Option<String>,
    path: Option<PathBuf>,
    registry: Option<String>,
}

impl Added {
    /// The value half of the `[dependencies]` entry, given a requirement.
    fn entry(&self, requirement: Option<&str>) -> Result<String, String> {
        let mut keys = Vec::new();
        if let Some(path) = &self.path {
            keys.push(format!("path = \"{}\"", path.display()));
        }
        if let Some(url) = &self.git {
            keys.push(format!("git = \"{url}\""));
            for (name, value) in [
                ("branch", &self.branch),
                ("tag", &self.tag),
                ("rev", &self.rev),
            ] {
                if let Some(value) = value {
                    keys.push(format!("{name} = \"{value}\""));
                }
            }
        }
        if let Some(registry) = &self.registry {
            if self.git.is_some() || self.path.is_some() {
                return Err(
                    "`--registry` names where to take a package from, and so do `--git` and `--path`; pick one"
                        .to_owned(),
                );
            }
            keys.push(format!("registry = \"{registry}\""));
        }
        if let Some(requirement) = requirement {
            // A bare requirement is the whole entry when nothing else was
            // asked for, which is the form most manifests want.
            if keys.is_empty() {
                return Ok(format!("\"{requirement}\""));
            }
            keys.insert(0, format!("version = \"{requirement}\""));
        } else if keys.is_empty() {
            return Err(
                "no source and no version requirement; write `name@<requirement>`, or give `--git`, `--path` or `--registry`"
                    .to_owned(),
            );
        }
        Ok(format!("{{ {} }}", keys.join(", ")))
    }
}

/// Write a dependency into a member's `Slopium.toml`, then resolve it.
///
/// The manifest is edited as text rather than reprinted from its parse: a
/// manifest is something a person wrote, and a tool that reformats it every
/// time it touches it is one people stop using.
fn add(
    manifest_path: Option<PathBuf>,
    spec: &str,
    added: Added,
    args: ResolveArgs,
    select: &SelectArgs,
) -> Result<(), String> {
    let (name, requirement) = match spec.split_once('@') {
        Some((name, requirement)) => (name, Some(requirement)),
        None => (spec, None),
    };
    validate_package_name(name)?;
    let entry = added.entry(requirement)?;

    let workspace = open_workspace(manifest_path.clone())?;
    let project = select.one(&workspace, "add")?;
    let edited = with_dependency(&project.manifest_source, name, Some(&entry))?;
    fs::write(&project.manifest_path, edited).map_err(|error| {
        format!(
            "cannot write `{}`: {error}",
            project.manifest_path.display()
        )
    })?;

    // Resolve afterwards, so a dependency that cannot be resolved is reported
    // by the same machinery every other command reports it with.
    let session = Session::open(manifest_path, args)?;
    let member = session.resolution.member(&project.name)?;
    match member.packages.get(name) {
        Some(package) => println!("Added {} {}", package.id, source_label(&package.id.source)),
        None => println!("Added `{name}`"),
    }
    Ok(())
}

fn remove(
    manifest_path: Option<PathBuf>,
    name: &str,
    args: ResolveArgs,
    select: &SelectArgs,
) -> Result<(), String> {
    let workspace = open_workspace(manifest_path.clone())?;
    let project = select.one(&workspace, "remove")?;
    if !project.dependencies.contains_key(name) {
        return Err(format!(
            "`{}` declares no dependency `{name}`",
            project.name
        ));
    }
    let edited = with_dependency(&project.manifest_source, name, None)?;
    fs::write(&project.manifest_path, edited).map_err(|error| {
        format!(
            "cannot write `{}`: {error}",
            project.manifest_path.display()
        )
    })?;
    Session::open(manifest_path, args)?;
    println!("Removed `{name}`");
    Ok(())
}

fn update(
    manifest_path: Option<PathBuf>,
    packages: Vec<String>,
    precise: Option<String>,
    args: ResolveArgs,
) -> Result<(), String> {
    if args.locked() {
        return Err(
            "SL1082: `update` is what moves the lockfile, and `--locked` forbids moving it"
                .to_owned(),
        );
    }
    if precise.is_some() && packages.len() != 1 {
        return Err(
            "`--precise` names one version, so it takes exactly one `-p <name>`".to_owned(),
        );
    }
    let update = Update {
        precise: precise.map(|text| Version::parse(&text)).transpose()?,
        packages,
    };
    let before = updatable_lock(manifest_path.clone())?;
    let session = Session::open_updating(manifest_path, args, &update)?;

    let mut moved = false;
    for (name, package) in &session.resolution.packages {
        let was = before.get(name);
        if was != Some(&package.id.version) {
            moved = true;
            match was {
                Some(version) => println!("Updated {name} v{version} -> v{}", package.id.version),
                None => println!("Added {}", package.id),
            }
        }
    }
    for (name, version) in &before {
        if !session.resolution.packages.contains_key(name) {
            moved = true;
            println!("Removed {name} v{version}");
        }
    }
    if !moved {
        println!("Everything is already at the newest version its requirements allow");
    }
    Ok(())
}

/// What the lockfile pinned before an update, so the report can say what moved.
fn updatable_lock(manifest_path: Option<PathBuf>) -> Result<BTreeMap<String, Version>, String> {
    let workspace = open_workspace(manifest_path)?;
    let Ok(text) = fs::read_to_string(workspace.lock_path()) else {
        return Ok(BTreeMap::new());
    };
    let Ok(lock) = Lockfile::parse(&text) else {
        return Ok(BTreeMap::new());
    };
    Ok(lock
        .packages
        .into_iter()
        .map(|package| (package.name, package.version))
        .collect())
}

/// A manifest with one `[dependencies]` entry written, replaced, or taken out.
///
/// Line-based on purpose. TOML keeps an inline table on one line, so an entry
/// this tool ever writes is one line; an entry written as `[dependencies.name]`
/// is not, and is refused rather than half-edited.
/// Add a path to `[workspace] members`, keeping the file readable.
///
/// Text rather than a TOML round trip, for the same reason `with_dependency` is
/// text: a manifest somebody wrote keeps its comments, its ordering and its
/// spacing. Only the one-line array form is written; anything else is refused
/// by name rather than half-edited.
fn with_member(source: &str, path: &str) -> Result<String, String> {
    let mut lines: Vec<String> = source.lines().map(str::to_owned).collect();
    let mut in_workspace = false;
    let mut end_of_workspace = None;
    for index in 0..lines.len() {
        let trimmed = lines[index].trim();
        if trimmed.starts_with('[') {
            if in_workspace {
                end_of_workspace = Some(index);
            }
            in_workspace = trimmed == "[workspace]";
            continue;
        }
        if !in_workspace || trimmed.split_once('=').map(|(key, _)| key.trim()) != Some("members") {
            continue;
        }
        let value = trimmed
            .split_once('=')
            .expect("the line holds a `=`")
            .1
            .trim();
        let inner = value
            .strip_prefix('[')
            .and_then(|value| value.strip_suffix(']'))
            .ok_or_else(|| {
                "`[workspace] members` is not written as a one-line array; add the package to it by hand"
                    .to_owned()
            })?;
        let inner = inner.trim().trim_end_matches(',').trim_end();
        let indent = &lines[index][..lines[index].len() - lines[index].trim_start().len()];
        lines[index] = match inner.is_empty() {
            true => format!("{indent}members = [\"{path}\"]"),
            false => format!("{indent}members = [{inner}, \"{path}\"]"),
        };
        return Ok(joined(&lines));
    }

    // `[workspace]` with no `members` at all: start the list at the end of the
    // table, not at the end of the file, or it would land in whatever table
    // comes next.
    let written = format!("members = [\"{path}\"]");
    match end_of_workspace.or(in_workspace.then_some(lines.len())) {
        Some(index) => {
            let insert = lines[..index]
                .iter()
                .rposition(|line| !line.trim().is_empty())
                .map(|last| last + 1)
                .unwrap_or(index);
            lines.insert(insert, written);
        }
        None => return Err("this manifest has no `[workspace]` table".to_owned()),
    }
    Ok(joined(&lines))
}

fn with_dependency(source: &str, name: &str, entry: Option<&str>) -> Result<String, String> {
    let section = format!("[dependencies.{name}]");
    if source.lines().any(|line| line.trim() == section) {
        return Err(format!(
            "`{name}` is written as `{section}`; edit it by hand — this command only writes the one-line form"
        ));
    }

    let mut lines: Vec<String> = source.lines().map(str::to_owned).collect();
    let is_entry = |line: &str| {
        line.split_once('=')
            .is_some_and(|(key, _)| key.trim().trim_matches('"') == name)
    };
    let mut in_dependencies = false;
    let mut end_of_dependencies = None;
    for index in 0..lines.len() {
        let trimmed = lines[index].trim();
        if trimmed.starts_with('[') {
            if in_dependencies {
                end_of_dependencies = Some(index);
            }
            in_dependencies = trimmed == "[dependencies]";
            continue;
        }
        if in_dependencies && is_entry(trimmed) {
            match entry {
                Some(entry) => lines[index] = format!("{name} = {entry}"),
                None => {
                    lines.remove(index);
                }
            }
            return Ok(joined(&lines));
        }
    }
    let Some(entry) = entry else {
        return Err(format!("`[dependencies]` has no `{name}` to remove"));
    };

    // Not there yet. Append to `[dependencies]`, or start the table.
    let written = format!("{name} = {entry}");
    match end_of_dependencies.or(in_dependencies.then_some(lines.len())) {
        Some(index) => {
            let insert = lines[..index]
                .iter()
                .rposition(|line| !line.trim().is_empty())
                .map(|last| last + 1)
                .unwrap_or(index);
            lines.insert(insert, written);
        }
        None => {
            while lines.last().is_some_and(|line| line.trim().is_empty()) {
                lines.pop();
            }
            lines.push(String::new());
            lines.push("[dependencies]".to_owned());
            lines.push(written);
        }
    }
    Ok(joined(&lines))
}

fn joined(lines: &[String]) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// Where a package comes from, for the one line `tree` gives it.
fn source_label(source: &SourceId) -> String {
    match source {
        SourceId::Path(_) => "(path)".to_owned(),
        SourceId::Toolchain => "(toolchain)".to_owned(),
        SourceId::Git { url, rev, .. } => format!("(git {url}#{})", &rev[..7.min(rev.len())]),
        SourceId::Registry { index } => format!("(registry {index})"),
    }
}

fn tree(
    session: &Session,
    project: &Project,
    depth: Option<usize>,
    duplicates: bool,
) -> Result<(), String> {
    #[allow(clippy::too_many_arguments)]
    fn walk(
        name: &str,
        resolution: &Resolution,
        prefix: &str,
        last: bool,
        root: bool,
        seen: &mut HashSet<String>,
        level: usize,
        limit: Option<usize>,
    ) {
        let Some(package) = resolution.packages.get(name) else {
            return;
        };
        let repeated = !seen.insert(name.to_owned());
        // A subtree that exists and is not shown says so. `--depth` is the
        // caller's own doing, but a tree that looks complete and is not is how
        // somebody concludes a dependency is absent.
        let elided = limit == Some(level) && !package.dependencies.is_empty() && !repeated;
        let mark = match (repeated && !package.dependencies.is_empty(), elided) {
            (true, _) => " (*)",
            (_, true) => " (...)",
            _ => "",
        };
        if root {
            println!("{}{mark}", package.id);
        } else {
            println!(
                "{prefix}{}{} {}{mark}",
                if last { "`-- " } else { "|-- " },
                package.id,
                source_label(&package.id.source),
            );
        }
        if repeated || elided {
            return;
        }
        let child_prefix = if root {
            String::new()
        } else {
            format!("{prefix}{}", if last { "    " } else { "|   " })
        };
        for (index, dependency) in package.dependencies.iter().enumerate() {
            let last = index + 1 == package.dependencies.len();
            walk(
                dependency,
                resolution,
                &child_prefix,
                last,
                false,
                seen,
                level + 1,
                limit,
            );
        }
    }

    let resolution = session.resolution.member(&project.name)?;
    if duplicates {
        return shared_dependencies(resolution);
    }
    walk(
        &resolution.root.name,
        resolution,
        "",
        true,
        true,
        &mut HashSet::new(),
        0,
        depth,
    );
    Ok(())
}

/// The packages more than one package in this graph depends on.
///
/// Not what `--duplicates` means to Cargo, and it cannot be: identity here is
/// name *and* version, and two versions of one name in a graph is an error
/// (`D-035`, `D-036`), so a second copy of a package is a thing this resolver
/// refuses rather than a thing to report. What is worth reporting is the other
/// kind of duplicate — the package reached along more than one path, which the
/// tree marks `(*)` and this lists with the dependents that share it.
fn shared_dependencies(resolution: &Resolution) -> Result<(), String> {
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for package in resolution.packages.values() {
        for dependency in &package.dependencies {
            dependents
                .entry(dependency.as_str())
                .or_default()
                .push(package.id.name.as_str());
        }
    }
    let shared = dependents
        .iter()
        .filter(|(_, dependents)| dependents.len() > 1)
        .collect::<Vec<_>>();
    if shared.is_empty() {
        println!("No dependency of {} is shared.", resolution.root);
        return Ok(());
    }
    for (name, dependents) in shared {
        let Some(package) = resolution.packages.get(*name) else {
            continue;
        };
        println!("{} {}", package.id, source_label(&package.id.source));
        for dependent in dependents {
            println!("|-- required by {dependent}");
        }
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

/// The runtime units this build links, written into `out_dir`.
///
/// There is more than one since `D-066` — a core half a freestanding program
/// could have alone, and a hosted half that supplies what libc is behind — so
/// this returns the set rather than the file. Each is rewritten only when its
/// bytes differ, because the timestamps feed the artifact cache.
///
/// Which half a build gets is the environment's to say, and the environment is
/// the target's. A freestanding build materializes the core half alone, and
/// because `cache_key` hashes the units it is handed rather than their names,
/// the key changes when the set shrinks without being told to.
fn materialize_runtime(out_dir: &Path, environment: Environment) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for (name, bytes) in slopic_core::runtime_sources(environment) {
        let path = out_dir.join(name);
        if fs::read(&path).ok().as_deref() != Some(bytes) {
            fs::write(&path, bytes)
                .map_err(|error| format!("cannot materialize `{}`: {error}", path.display()))?;
        }
        paths.push(path);
    }
    Ok(paths)
}

fn source_path(project: &Project) -> Result<PathBuf, String> {
    let source = project.entry_path()?;
    if !source.is_file() {
        return Err(format!(
            "entry source `{}` does not exist",
            source.display()
        ));
    }
    Ok(source)
}

fn source_files(project: &Project) -> Result<Vec<PathBuf>, String> {
    let root = project.source_root()?;
    let mut files = Vec::new();
    collect_cache_sources(&root, &mut files)?;
    files.sort();
    Ok(files)
}

/// A resolved package as this build needs it: a namespace and a place to read
/// modules from.
///
/// The resolver returns package identities; codegen and the cache want source
/// roots. This is the adapter between the two, and the only place that knows a
/// namespace is a package name (`D-035`).
#[derive(Clone, Debug)]
enum ResolvedDependencySource {
    Path(PathBuf),
    Toolchain,
}

#[derive(Clone, Debug)]
struct ResolvedDependency {
    namespace: String,
    source: ResolvedDependencySource,
    manifest_source: Option<String>,
    /// The modules of this dependency the selected target leaves out
    /// (`D-135`). A library with a module per target is what this exists for.
    excluded_modules: Vec<String>,
    /// The file each of this dependency's named modules is, for this target.
    named_modules: Vec<(String, PathBuf)>,
}

/// Everything resolution produced that the rest of the build consumes.
#[derive(Clone, Debug, Default)]
struct Dependencies {
    packages: Vec<ResolvedDependency>,
    language_items: Vec<(String, String)>,
    /// Every dependency's `c-sources`, made absolute. A library that declares
    /// an `extern` carries the C that defines it, and the link that consumes
    /// the library is the one that has to compile it (`D-075`).
    c_sources: Vec<PathBuf>,
}

/// Turn one member's resolved graph into what its build consumes.
fn dependencies_of(
    project: &Project,
    resolution: &Resolution,
    target: &str,
) -> Result<Dependencies, String> {
    reject_namespace_collisions(project, resolution)?;

    let mut packages = Vec::new();
    let mut c_sources = Vec::new();
    for package in resolution.dependencies() {
        if let Some(project) = &package.project {
            c_sources.extend(c_source_paths(project));
        }
        let source = match (&package.id.source, &package.project) {
            // A vendored copy of the bundled library is an ordinary directory
            // of sources, and the compiler is handed it as one. What makes it
            // the standard library is the language items it declares, not where
            // the bytes came from (`D-011`).
            (SourceId::Toolchain, Some(project)) => {
                ResolvedDependencySource::Path(project.source_root()?)
            }
            (SourceId::Toolchain, None) => ResolvedDependencySource::Toolchain,
            // A fetched package has been unpacked into the store or copied into
            // the vendor directory by the time a build reads it, so like every
            // other package it is a directory of sources.
            (
                SourceId::Path(_) | SourceId::Git { .. } | SourceId::Registry { .. },
                Some(project),
            ) => ResolvedDependencySource::Path(project.source_root()?),
            (source, None) => {
                return Err(format!(
                    "package `{}` from `{source}` was resolved without a manifest",
                    package.id.name
                ))
            }
        };
        let selection = match &package.project {
            Some(project) => target_selection(project, target)?,
            None => TargetSelection::default(),
        };
        packages.push(ResolvedDependency {
            namespace: package.namespace().to_owned(),
            source,
            manifest_source: package
                .project
                .as_ref()
                .map(|project| project.manifest_source.clone()),
            excluded_modules: selection.excluded,
            named_modules: selection.named,
        });
    }
    Ok(Dependencies {
        packages,
        language_items: resolution.language_items.clone(),
        c_sources,
    })
}

/// A package's `c-sources`, resolved against its root.
///
/// The manifest holds them relative and refuses anything that leaves the
/// package, so joining is the whole of it.
fn c_source_paths(project: &Project) -> Vec<PathBuf> {
    project
        .c_sources
        .iter()
        .map(|path| project.root.join(path))
        .collect()
}

/// What `[target."<triple>"]` decides about one package's own modules
/// (`D-135`).
#[derive(Default)]
struct TargetSelection {
    /// Modules left out, by the name the compiler would have derived from the
    /// path — which is how it knows them, since it names a module by where it
    /// found it.
    excluded: Vec<String>,
    /// The file each named module is, for the target being built. The compiler
    /// is told the name because the path no longer decides it.
    named: Vec<(String, PathBuf)>,
}

/// Reads `[target."<triple>"]` for one package and one target (`D-135`).
///
/// The manifest says what each module *is* per target. Every file any target
/// names is out of the build unless the target naming it is the one selected,
/// and the file that is selected is handed over under the name the manifest
/// gave it rather than the one its path would have produced. That is what lets
/// the rest of a program write `(take arch ...)` once.
///
/// A target with no table of its own gets none of these files, which is the
/// honest reading of "exactly one of them is in the build" and needs no list of
/// known triples: a table for a target this toolchain cannot build for simply
/// never names the selected one.
///
/// A path that names nothing is an error rather than a selection that quietly
/// does nothing, the standard `D-128` set for a key nobody knows.
fn target_selection(project: &Project, target: &str) -> Result<TargetSelection, String> {
    if project.target_modules.is_empty() {
        return Ok(TargetSelection::default());
    }
    let source_root = project.source_root()?;
    let mut selection = TargetSelection::default();
    let mut selected_paths = Vec::new();
    for (triple, modules) in &project.target_modules {
        for (module, path) in modules {
            let absolute = project.root.join(path);
            if !absolute.is_file() {
                return Err(format!(
                    "SL1102: `target.{triple}` module `{module}` of package `{}` names no file \
                     at `{}`",
                    project.name,
                    path.display()
                ));
            }
            let absolute = absolute
                .canonicalize()
                .map_err(|error| format!("cannot resolve `{}`: {error}", absolute.display()))?;
            if !absolute.starts_with(&source_root) {
                return Err(format!(
                    "SL1102: `target.{triple}` module `{module}` of package `{}` is not under \
                     its source root",
                    project.name
                ));
            }
            if triple == target {
                selection.named.push((module.clone(), absolute.clone()));
                selected_paths.push(absolute);
            }
        }
    }
    for modules in project.target_modules.values() {
        for path in modules.values() {
            let absolute = project.root.join(path);
            let Ok(absolute) = absolute.canonicalize() else {
                continue;
            };
            if selected_paths.contains(&absolute) {
                continue;
            }
            let Some(module) = slopic_core::module_from_path(&source_root, &absolute) else {
                continue;
            };
            selection.excluded.push(module);
        }
    }
    selection.excluded.sort();
    selection.excluded.dedup();
    selection.named.sort();
    selection.named.dedup();
    Ok(selection)
}

/// A package's `[build] linker-script`, resolved against its root.
///
/// Only the root's is ever asked for. A dependency's `[build]` is not consulted
/// for `target` either, and for the same reason: it describes the program being
/// built, and a dependency is not building it (`D-117`).
fn linker_script_path(project: &Project) -> Option<PathBuf> {
    project
        .linker_script
        .as_ref()
        .map(|path| project.root.join(path))
}

/// The name `slopium vendor` gives the replacement source it writes.
const VENDOR_SOURCE: &str = "vendored";

/// Write a package archive and say what it hashes to.
///
/// The digest is the package's name for every purpose that matters — the lock
/// records it, the store files the bytes under it, and a vendored copy is
/// checked against it — so it is printed in `sha256sum` order, digest first, to
/// be compared against that tool directly.
fn package(workspace: &Workspace, project: &Project, index_entry: bool) -> Result<PathBuf, String> {
    let (bytes, digest) = package_archive(project)?;
    let directory = workspace.target_dir().join("package");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("cannot create `{}`: {error}", directory.display()))?;
    let path = directory.join(format!(
        "{}-{}.{ARCHIVE_EXTENSION}",
        project.name, project.version
    ));
    fs::write(&path, &bytes)
        .map_err(|error| format!("cannot write `{}`: {error}", path.display()))?;
    if index_entry {
        println!("{}", published_entry(project, digest)?.render()?);
    } else {
        println!("Packaged {} v{}", project.name, project.version);
        println!("{digest}  {}", path.display());
    }
    Ok(path)
}

/// Sign a package and put it in a registry (`D-059`).
///
/// This is `package` plus a signature plus three file writes, and deliberately
/// nothing more. A registry is a directory somebody serves, so publishing is
/// putting files in it; what an `https://` index needs instead is for its host
/// to put the same files in the same places.
fn publish(
    session: &Session,
    project: &Project,
    key: &Path,
    registry: Option<&str>,
    dry_run: bool,
) -> Result<(), String> {
    let name = registry.unwrap_or(DEFAULT_REGISTRY);
    let registry = session.sources.registries().named(name)?;
    let root = registry.directory().ok_or_else(|| {
        format!(
            "registry `{name}` is the index `{}`, and only a directory can be published to; there is no upload protocol because there is no server (`D-059`). Put the files where that index serves them",
            registry.index()
        )
    })?.to_owned();

    let (bytes, digest) = package_archive(project)?;
    round_trips(&bytes, &digest)?;
    // Before the key is even read: a manifest that cannot become an index entry
    // is one no signature would make publishable (`SL1032`).
    let mut entry = published_entry(project, digest)?;
    if registry
        .versions(&project.name)?
        .iter()
        .any(|published| published.version == project.version)
    {
        return Err(format!(
            "SL1043: `{}` already publishes {} v{}. An index line is append-only, because somebody's lock may already name that version and a republished one is the change no lock can notice; publish {} instead",
            registry.index(),
            project.name,
            project.version,
            next_version(&project.version)
        ));
    }
    let signature = PrivateKey::read(key)?.sign(&project.name, &project.version, &digest);
    entry.signature = Some(signature);
    let line = entry.render()?;

    let archive = root.join(published_archive_path(&project.name, &project.version));
    let detached = root.join(published_signature_path(&project.name, &project.version));
    let index = root
        .join(INDEX_DIRECTORY)
        .join(published_index_path(&project.name)?);
    if dry_run {
        println!("Would publish {} v{}", project.name, project.version);
        println!("  {}  {digest}", archive.display());
        println!("  {}  {signature}", detached.display());
        println!("  {}", index.display());
        println!("{line}");
        return Ok(());
    }

    write_under(&archive, &bytes)?;
    write_under(&detached, format!("{signature}\n").as_bytes())?;
    fs::create_dir_all(index.parent().expect("an index file has a directory"))
        .map_err(|error| format!("cannot create `{}`: {error}", index.display()))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&index)
        .map_err(|error| format!("cannot open `{}`: {error}", index.display()))?;
    writeln!(file, "{line}")
        .map_err(|error| format!("cannot write `{}`: {error}", index.display()))?;

    println!("Published {} v{}", project.name, project.version);
    println!("{digest}  {}", archive.display());
    println!("Signed by {}", signature.claimed_key());
    Ok(())
}

/// Require an archive to survive being unpacked and packed again.
///
/// The format was specified so that this holds (`D-039`), and the moment before
/// a signature asserts that these bytes are the package is the moment to find
/// out that it does.
fn round_trips(bytes: &[u8], digest: &Digest) -> Result<(), String> {
    let again = slopium_manifest::archive::write(&slopium_manifest::archive::read(bytes)?)?;
    if again == bytes {
        return Ok(());
    }
    Err(format!(
        "SL1004: the archive of this package does not reproduce itself: unpacking {digest} and packing it again gives {}. It would be signed as one thing and arrive as another",
        slopium_manifest::sha256::sha256(&again)
    ))
}

fn write_under(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let directory = path.parent().expect("a published file has a directory");
    fs::create_dir_all(directory)
        .map_err(|error| format!("cannot create `{}`: {error}", directory.display()))?;
    fs::write(path, bytes).map_err(|error| format!("cannot write `{}`: {error}", path.display()))
}

/// The next version to suggest when one is already published.
fn next_version(version: &Version) -> Version {
    Version::new(version.major, version.minor, version.patch + 1)
}

/// Re-check every dependency the store holds.
///
/// It goes through the same checkout every build goes through, so what it
/// verifies is exactly what a build would use — and on a machine whose store is
/// empty it fills it, which is what makes this the command to run first in a
/// fresh checkout.
fn verify(session: &Session) -> Result<(), String> {
    let mut checked = 0;
    for package in session.resolution.packages.values() {
        // A path dependency is a working tree with no checksum to be against,
        // and saying so once at the end beats a line per directory.
        let Some(checksum) = package.checksum else {
            continue;
        };
        session.sources.checkout(&package.id, &checksum)?;
        let signed = match session.sources.store().signature(&checksum)? {
            Some(signature) => format!("signed by {}", signature.claimed_key()),
            None => "unsigned".to_owned(),
        };
        println!("{}  {checksum}  {signed}", package.id);
        checked += 1;
    }
    println!("Verified {checked} package(s) against `{LOCK_FILE}`.");
    Ok(())
}

/// Write a signing key and print the half that goes into a configuration file.
fn new_key(path: &Path) -> Result<(), String> {
    let key = PrivateKey::generate()?;
    key.write(path)?;
    println!("Wrote `{}` at mode 0600. Back it up: a package published under a key nobody has any more can never be published again under the same one.", path.display());
    println!();
    println!("[registry.default]");
    println!("trusted-keys = [\"{}\"]", key.public());
    Ok(())
}

/// The index line describing an archive of this package.
///
/// This is where `D-054` is enforced from the writing side: what a published
/// package may depend on is what an index entry can say, so a manifest that
/// depends on a directory or a repository cannot become one.
fn published_entry(project: &Project, checksum: Digest) -> Result<IndexEntry, String> {
    let mut dependencies = Vec::new();
    for (name, spec) in &project.dependencies {
        let unpublishable = |what: &str| {
            Err(format!(
                "SL1032: `{}` depends on `{name}` through {what}; a published package depends only on its own registry and the toolchain (`D-054`)",
                project.name
            ))
        };
        let source = match spec.source(name)? {
            SourceSpec::Toolchain => IndexSource::Toolchain,
            SourceSpec::Registry { registry } if registry == DEFAULT_REGISTRY => {
                IndexSource::SameIndex
            }
            // A registry's local nickname means nothing to whoever reads the
            // entry, and there is no other way for the entry to say it.
            SourceSpec::Registry { registry } => {
                return unpublishable(&format!("the registry it calls `{registry}`"))
            }
            SourceSpec::Path(_) => return unpublishable("a directory"),
            SourceSpec::Git { .. } => return unpublishable("a repository"),
        };
        dependencies.push(IndexDependency {
            name: name.clone(),
            requirement: spec.requirement(),
            source,
        });
    }
    Ok(IndexEntry {
        name: project.name.clone(),
        version: project.version.clone(),
        dependencies,
        checksum,
        yanked: false,
        signature: None,
    })
}

/// Copy every dependency that is not a directory on this machine into the
/// vendor directory, and point builds at it.
///
/// Only packages with immutable bytes are vendored, which is what having a
/// checksum means: a path dependency is already a directory on this machine and
/// copying it would only make a second one that can drift from the first.
fn vendor(session: &Session, directory: &Path, member: Option<&str>) -> Result<(), String> {
    let workspace = &session.workspace;
    let root = workspace.root.join(directory);
    let mut vendored = Vec::new();

    // Everything the workspace resolves, or one member's share of it. The lock
    // covers the whole workspace either way, so `-p` narrows what is copied and
    // never what was resolved.
    let selected = match member {
        None => session.resolution.packages.values().collect::<Vec<_>>(),
        Some(name) => {
            // Through the workspace first: `member` on the resolution reports a
            // package that was not resolved, which is not what a misspelled
            // `-p` is.
            workspace.member(name)?;
            session
                .resolution
                .member(name)?
                .packages
                .values()
                .collect::<Vec<_>>()
        }
    };
    let mut copied = HashSet::new();

    for package in selected {
        let Some(checksum) = package.checksum else {
            continue;
        };
        copied.insert(package.id.name.clone());
        let described = package.id.to_string();
        // Through the store rather than around it: the copy that lands in the
        // vendor directory is one that has already been verified against its
        // digest and unpacked by code that refuses to write outside itself.
        let checkout = session.sources.checkout(&package.id, &checksum)?;
        let destination = root.join(&package.id.name);
        remove_tree(&destination)?;
        copy_tree(&checkout, &destination)?;
        println!("Vendored {described} ({checksum})");
        vendored.push(package.id.source.config_name());
    }

    if vendored.is_empty() {
        println!("Nothing to vendor: every dependency is already a directory on this machine.");
        return Ok(());
    }
    report_members_left_out(session, &copied);
    vendored.sort_unstable();
    vendored.dedup();
    let name = directory
        .to_str()
        .ok_or_else(|| "the vendor directory is not a portable path".to_owned())?;
    redirect_sources(workspace, &vendored, name)
}

/// Say which members `vendor -p` has just stopped from building offline.
///
/// The redirection written below covers the whole workspace, because it names
/// sources rather than packages: after a partial copy, a member needing
/// something that was not copied looks for it in the vendor directory and does
/// not find it. That is a reasonable thing to want — a release copying one
/// member's dependencies and no others — but not a reasonable thing to discover
/// later.
fn report_members_left_out(session: &Session, copied: &HashSet<String>) {
    let left_out = session
        .resolution
        .members
        .iter()
        .filter(|(_, resolution)| {
            resolution
                .packages
                .values()
                .any(|package| package.checksum.is_some() && !copied.contains(&package.id.name))
        })
        .map(|(name, _)| format!("`{name}`"))
        .collect::<Vec<_>>();
    if left_out.is_empty() {
        return;
    }
    println!(
        "Note: {} still needs packages that were not copied, so it will not build `--offline` from this vendor directory.",
        left_out.join(", ")
    );
}

/// Whether every `[source]` table already there is a redirection to this very
/// vendor directory — that is, one `slopium vendor` could have written.
///
/// Shape rather than provenance: nothing records who wrote a config file, and a
/// hand-written redirection identical to ours is one there is no reason to
/// refuse. What is refused is a redirection pointing somewhere else, or a
/// `[source]` table doing something this command does not understand.
fn redirects_here(config: &LocalConfig, workspace: &Workspace, directory: &str) -> bool {
    let target = workspace.root.join(directory);
    let vendor = config.source.get(VENDOR_SOURCE);
    if vendor.and_then(|entry| entry.directory.as_ref()) != Some(&PathBuf::from(directory)) {
        return false;
    }
    config.source.iter().all(|(name, entry)| {
        name == VENDOR_SOURCE
            || config
                .replacement(name, &workspace.root)
                .ok()
                .flatten()
                .is_some_and(|replacement| {
                    replacement == target && entry.replace_with.as_deref() == Some(VENDOR_SOURCE)
                })
    })
}

/// Point `.slopium/config.toml` at the vendor directory.
///
/// The file belongs to the checkout and may already say things about the C
/// compiler, so this appends rather than replaces — and refuses outright if
/// sources are already configured by hand, because guessing which half of
/// somebody's redirection to keep is not something to do to their configuration.
///
/// A redirection this command wrote is a different matter, and `-p` makes it a
/// common one: vendoring one member and then the workspace wants a second and a
/// third source added to a file that already redirects the first. That case
/// appends what is missing instead of refusing, because there is nothing there
/// to lose.
fn redirect_sources(
    workspace: &Workspace,
    sources: &[&str],
    directory: &str,
) -> Result<(), String> {
    // Read from disk rather than from the workspace: `vendor` deliberately
    // resolves with replacements ignored, so the copy it holds says nothing
    // about what the file on disk contains.
    let config = load_local_config(&workspace.root)?;
    let configured = config.source.keys().cloned().collect::<Vec<_>>();
    let mut wanted = sources.to_vec();
    wanted.push(VENDOR_SOURCE);
    wanted.sort_unstable();

    let ours = redirects_here(&config, workspace, directory);
    let missing = wanted
        .iter()
        .filter(|source| !configured.iter().any(|name| name == *source))
        .copied()
        .collect::<Vec<_>>();
    if !configured.is_empty() {
        if !ours {
            return Err(format!(
                "`.slopium/config.toml` already configures {}; edit it by hand or remove the `[source]` tables and vendor again",
                configured
                    .iter()
                    .map(|name| format!("`[source.{name}]`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if missing.is_empty() {
            return Ok(());
        }
    }

    // Everything, or only what the existing redirection does not already cover.
    let (added, header) = match configured.is_empty() {
        true => (
            sources.to_vec(),
            format!(
                "\n# Written by `slopium vendor`. Builds read these packages from `{directory}`\n\
                 # instead of the source named; delete these tables to go back.\n"
            ),
        ),
        false => (
            missing
                .into_iter()
                .filter(|source| *source != VENDOR_SOURCE)
                .collect(),
            "\n# Added by a later `slopium vendor`.\n".to_owned(),
        ),
    };
    let mut text = header;
    for source in &added {
        text.push_str(&format!(
            "\n[source.{source}]\nreplace-with = \"{VENDOR_SOURCE}\"\n"
        ));
    }
    if !configured.is_empty() {
        let path = workspace.root.join(".slopium/config.toml");
        let existing = fs::read_to_string(&path)
            .map_err(|error| format!("cannot read `{}`: {error}", path.display()))?;
        fs::write(&path, format!("{existing}{text}"))
            .map_err(|error| format!("cannot write `{}`: {error}", path.display()))?;
        println!("Wrote {}", path.display());
        return Ok(());
    }
    text.push_str(&format!(
        "\n[source.{VENDOR_SOURCE}]\ndirectory = \"{directory}\"\n"
    ));

    let path = workspace.root.join(".slopium/config.toml");
    let parent = path.parent().expect("a config path has a parent");
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create `{}`: {error}", parent.display()))?;
    let existing = match fs::read_to_string(&path) {
        Ok(existing) => existing,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("cannot read `{}`: {error}", path.display())),
    };
    fs::write(&path, format!("{existing}{text}"))
        .map_err(|error| format!("cannot write `{}`: {error}", path.display()))?;
    println!("Wrote {}", path.display());
    Ok(())
}

/// Copy a checked-out tree into the vendor directory, writable.
///
/// The store keeps its files read-only so nobody edits a package by accident;
/// a vendored copy is part of the checkout and is left as ordinary files, which
/// is why it is verified against its checksum on every build rather than
/// protected by its permissions.
fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    fs::create_dir_all(to).map_err(|error| format!("cannot create `{}`: {error}", to.display()))?;
    for child in
        fs::read_dir(from).map_err(|error| format!("cannot read `{}`: {error}", from.display()))?
    {
        let child = child.map_err(|error| format!("cannot read `{}`: {error}", from.display()))?;
        let source = child.path();
        let destination = to.join(child.file_name());
        if source.is_dir() {
            copy_tree(&source, &destination)?;
            continue;
        }
        let bytes = fs::read(&source)
            .map_err(|error| format!("cannot read `{}`: {error}", source.display()))?;
        fs::write(&destination, bytes)
            .map_err(|error| format!("cannot write `{}`: {error}", destination.display()))?;
    }
    Ok(())
}

/// A dependency namespace and a local module namespace cannot both be spelled
/// the same way, because the compiler resolves them in one flat space.
fn reject_namespace_collisions(project: &Project, resolution: &Resolution) -> Result<(), String> {
    let source_root = project.source_root()?;
    let mut sources = Vec::new();
    collect_cache_sources(&source_root, &mut sources)?;
    let mut local_roots = HashSet::new();
    for source in sources {
        let relative = source.strip_prefix(&source_root).map_err(|error| {
            format!(
                "cannot map source `{}` relative to `{}`: {error}",
                source.display(),
                source_root.display()
            )
        })?;
        if let Some(first) = relative.components().next().and_then(|component| {
            Path::new(component.as_os_str())
                .file_stem()
                .and_then(|name| name.to_str())
        }) {
            local_roots.insert(first.to_owned());
        }
    }
    for package in resolution.dependencies() {
        if local_roots.contains(package.namespace()) {
            return Err(format!(
                "dependency `{}` collides with the local module namespace",
                package.namespace()
            ));
        }
    }
    Ok(())
}

/// Read the lockfile, which is what pins a source that would otherwise move.
fn read_lock(workspace: &Workspace, args: ResolveArgs) -> Result<Option<Lockfile>, String> {
    let path = workspace.lock_path();
    match fs::read_to_string(&path) {
        // A lock this toolchain cannot read is still only a build product, and
        // resolution can write a new one from the manifests alone. Saying so
        // and carrying on beats making somebody delete a file by hand — but
        // `--locked` asked for exactly the opposite, so there it is an error.
        Ok(text) => match Lockfile::parse(&text) {
            Ok(lock) => Ok(Some(lock)),
            Err(error) if args.locked() => Err(error),
            Err(error) => {
                println!("Replacing {LOCK_FILE}: {error}");
                Ok(None)
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("cannot read `{}`: {error}", path.display())),
    }
}

/// Write the workspace's lockfile, or refuse to under `--locked`.
fn synchronize_lock(
    workspace: &Workspace,
    resolution: &WorkspaceResolution,
    existing: Option<Lockfile>,
    args: ResolveArgs,
) -> Result<(), String> {
    let path = workspace.lock_path();
    let resolved = Lockfile::from_packages(&resolution.packages, &workspace.root);
    if existing.as_ref() == Some(&resolved) {
        return Ok(());
    }
    if args.locked() {
        return Err(match existing {
            Some(_) => format!(
                "SL1082: `{LOCK_FILE}` is out of date and --locked was given; run without it to update"
            ),
            None => format!("SL1082: `{LOCK_FILE}` is missing and --locked was given"),
        });
    }
    fs::write(&path, resolved.render())
        .map_err(|error| format!("cannot write `{}`: {error}", path.display()))
}

/// Tells the compiler which of the root package's modules this build is not
/// made of, and which file each named one is (`D-135`).
fn add_selection_args(command: &mut Command, selection: &TargetSelection) {
    for module in &selection.excluded {
        command.arg("--exclude-module").arg(module);
    }
    for (module, path) in &selection.named {
        command
            .arg("--module")
            .arg(format!("{module}={}", path.display()));
    }
}

fn add_dependency_args(command: &mut Command, dependencies: &Dependencies) {
    for dependency in &dependencies.packages {
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
        for module in &dependency.excluded_modules {
            command
                .arg("--dependency-exclude")
                .arg(format!("{}={module}", dependency.namespace));
        }
        for (module, path) in &dependency.named_modules {
            command.arg("--dependency-module").arg(format!(
                "{}={module}={}",
                dependency.namespace,
                path.display()
            ));
        }
    }
    for (name, path) in &dependencies.language_items {
        command.arg("--language-item").arg(format!("{name}={path}"));
    }
}

fn target(project: &Project, override_target: Option<String>) -> String {
    override_target
        .or_else(|| std::env::var("SLOPIUM_TARGET").ok())
        .or_else(|| project.manifest.build.target.clone())
        .unwrap_or_else(|| DEFAULT_TARGET.into())
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

/// Whether a profile optimizes. Any `opt-level` above zero does; the
/// conventional default is `release` only.
fn optimizes(profile: Option<&Profile>, profile_name: &str) -> bool {
    profile
        .and_then(|profile| profile.opt_level)
        .map(|level| level > 0)
        .unwrap_or(profile_name == "release")
}

/// Whether a profile strips the binary.
///
/// The default is the opposite of `debug`: a build you can debug keeps its
/// symbols, a build you ship does not. Stripping a debug build would remove
/// the line tables it exists to provide, so an explicit `strip = true` there
/// is honoured but defeats `debug`.
fn strip_symbols(profile: Option<&Profile>, profile_name: &str) -> bool {
    profile
        .and_then(|profile| profile.strip)
        .unwrap_or(!debug_info(profile, profile_name))
}

/// Whether a profile aborts on a trap without a message. The default is to keep
/// the message: a silent crash is a worse default than a few bytes of string.
fn panic_abort(profile: Option<&Profile>) -> bool {
    profile
        .and_then(|profile| profile.panic.as_deref())
        .map(|mode| mode == "abort")
        .unwrap_or(false)
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
        // A target's own default rather than a bare `cc`: the host driver
        // would accept the assembly and quietly produce host objects that
        // fail only at link time, with a message about the wrong
        // architecture rather than about the missing cross toolchain.
        .unwrap_or_else(|| {
            TARGETS
                .iter()
                .find(|spec| spec.triple == target)
                .map(|spec| spec.default_cc.to_owned())
                .unwrap_or_else(|| "cc".into())
        })
}

#[derive(Clone, Copy)]
struct CacheInputs<'a> {
    project: &'a Project,
    source_root: &'a Path,
    dependencies: &'a Dependencies,
    target: &'a str,
    profile_name: &'a str,
    profile: Option<&'a Profile>,
    test: bool,
    compiler: &'a Path,
    runtimes: &'a [PathBuf],
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
    for dependency in &input.dependencies.packages {
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
    hasher.write(&[u8::from(strip_symbols(input.profile, input.profile_name))]);
    hasher.write(&[u8::from(panic_abort(input.profile))]);
    let metadata = fs::metadata(input.compiler)
        .map_err(|error| format!("cannot inspect `{}`: {error}", input.compiler.display()))?;
    hasher.write(&metadata.len().to_le_bytes());
    if let Ok(modified) = metadata.modified() {
        if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
            hasher.write(&duration.as_nanos().to_le_bytes());
        }
    }
    for runtime in input.runtimes {
        hasher.write(
            &fs::read(runtime)
                .map_err(|error| format!("cannot hash `{}`: {error}", runtime.display()))?,
        );
    }
    // The manifest is hashed above, so *declaring* a `c-sources` entry already
    // invalidates the artifact. Their contents are hashed here so that editing
    // one does too — `collect_cache_sources` walks only `.slp` (`D-075`).
    for c_source in c_source_paths(input.project)
        .into_iter()
        .chain(input.dependencies.c_sources.iter().cloned())
    {
        hasher.write(c_source.display().to_string().as_bytes());
        hasher.write(
            &fs::read(&c_source)
                .map_err(|error| format!("cannot hash `{}`: {error}", c_source.display()))?,
        );
    }
    // And the linker script for the same reason: renaming it is a manifest edit
    // the text above catches, and editing it is not.
    if let Some(script) = linker_script_path(input.project) {
        hasher.write(script.display().to_string().as_bytes());
        hasher.write(
            &fs::read(&script)
                .map_err(|error| format!("cannot hash `{}`: {error}", script.display()))?,
        );
    }
    hasher.write(input.cc.as_bytes());
    Ok(format!("{:016x}", hasher.finish()))
}

fn module_cache_key(
    input: CacheInputs<'_>,
    unit: &ModuleCacheUnit,
    units: &[ModuleCacheUnit],
) -> Result<String, String> {
    let mut hasher = Fnv1a::default();
    hasher.write(b"slopium-object-cache-v3");
    hasher.write(input.project.manifest_source.as_bytes());
    hasher.write(input.target.as_bytes());
    hasher.write(input.profile_name.as_bytes());
    hasher.write(&[u8::from(input.test)]);
    if let Some(profile) = input.profile {
        hasher.write(&[profile.opt_level.unwrap_or_default()]);
    }
    hasher.write(&[u8::from(debug_info(input.profile, input.profile_name))]);
    hasher.write(&[u8::from(strip_symbols(input.profile, input.profile_name))]);
    hasher.write(&[u8::from(panic_abort(input.profile))]);
    let metadata = fs::metadata(input.compiler)
        .map_err(|error| format!("cannot inspect `{}`: {error}", input.compiler.display()))?;
    hasher.write(&metadata.len().to_le_bytes());
    if let Ok(modified) = metadata.modified() {
        if let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH) {
            hasher.write(&duration.as_nanos().to_le_bytes());
        }
    }
    hasher.write(input.cc.as_bytes());
    for dependency in &input.dependencies.packages {
        hasher.write(dependency.namespace.as_bytes());
        if let Some(manifest) = &dependency.manifest_source {
            hasher.write(manifest.as_bytes());
        }
    }
    for (name, path) in &input.dependencies.language_items {
        hasher.write(name.as_bytes());
        hasher.write(path.as_bytes());
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
            "SL1090: incompatible slopic protocol {}; slopium requires {}",
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

/// The summary line for a `slopic` invocation that emits one module's object.
///
/// The compiler exits `1` when the program does not compile, having printed
/// every diagnostic itself, and `2` when anything else goes wrong. Naming a
/// module in the first case is worse than saying nothing: a build asks for one
/// object per invocation and each invocation checks the whole program first, so
/// an error anywhere fails all of them and the loop would name whichever
/// module's object it happened to be asking for — the first in the order on a
/// cold build, the first stale one afterwards. A reader who trusts that line
/// goes looking in the standard library for a bug in the file above it
/// (`D-154`). A module name is kept for the other statuses, which is the case
/// the message was written for and the only one where the name carries
/// information.
fn codegen_status_result(status: ExitStatus, module: &str) -> Result<(), String> {
    if status.code() == Some(1) {
        return status_result(status, "build");
    }
    status_result(status, &format!("codegen for module `{module}`"))
}

fn status_result(status: ExitStatus, action: &str) -> Result<(), String> {
    if status.success() {
        Ok(())
    } else {
        Err(format!("{action} failed with {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slopium_manifest::workspace::load_project;

    const MANIFEST: &str = "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\nstd = { toolchain = true }\n\n[build]\ntarget = \"x86_64-unknown-linux-gnu\"\n";

    /// `add` edits the file rather than reprinting it, so everything it did not
    /// touch comes out byte-identical.
    #[test]
    fn adding_a_dependency_leaves_the_rest_of_the_manifest_alone() {
        let edited = with_dependency(MANIFEST, "geometry", Some("\"^1.2\"")).unwrap();
        assert_eq!(
            edited,
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\nstd = { toolchain = true }\ngeometry = \"^1.2\"\n\n[build]\ntarget = \"x86_64-unknown-linux-gnu\"\n"
        );
    }

    #[test]
    fn adding_a_dependency_twice_replaces_it() {
        let once = with_dependency(MANIFEST, "geometry", Some("\"^1.2\"")).unwrap();
        let twice = with_dependency(&once, "geometry", Some("\"^2\"")).unwrap();
        assert!(twice.contains("geometry = \"^2\""), "{twice}");
        assert!(!twice.contains("^1.2"), "{twice}");
    }

    #[test]
    fn removing_a_dependency_takes_out_its_line() {
        let removed = with_dependency(MANIFEST, "std", None).unwrap();
        assert!(!removed.contains("toolchain"), "{removed}");
        assert!(removed.contains("[dependencies]"), "{removed}");
        assert!(with_dependency(&removed, "std", None).is_err());
    }

    /// A manifest with no `[dependencies]` gets one, at the end where a person
    /// would have put it.
    #[test]
    fn a_manifest_without_dependencies_grows_the_table() {
        let edited = with_dependency(
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n",
            "geometry",
            Some("\"^1\""),
        )
        .unwrap();
        assert_eq!(
            edited,
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\ngeometry = \"^1\"\n"
        );
    }

    /// `slopium new` inside a workspace has to leave the root manifest
    /// buildable, and readable: comments and ordering stay where they were.
    #[test]
    fn a_member_is_appended_to_the_list_that_is_there() {
        let source = "[workspace]\nmembers = [\"alpha\"]\n\n# kept\n[workspace.package]\nversion = \"0.1.0\"\n";
        let edited = with_member(source, "beta").unwrap();
        assert!(
            edited.contains("members = [\"alpha\", \"beta\"]"),
            "{edited}"
        );
        assert!(edited.contains("# kept"), "{edited}");
    }

    /// An empty list, a trailing comma, and a `[workspace]` with no `members`
    /// at all are the three shapes a hand-written root turns up in.
    #[test]
    fn a_member_list_is_started_when_there_is_none() {
        assert!(with_member("[workspace]\nmembers = []\n", "beta")
            .unwrap()
            .contains("members = [\"beta\"]"));
        assert!(with_member("[workspace]\nmembers = [\"alpha\",]\n", "beta")
            .unwrap()
            .contains("members = [\"alpha\", \"beta\"]"));
        // The list has to land inside `[workspace]`, not in the table below it.
        let edited = with_member(
            "[workspace]\n\n[workspace.package]\nversion = \"0.1.0\"\n",
            "beta",
        )
        .unwrap();
        assert_eq!(
            edited,
            "[workspace]\nmembers = [\"beta\"]\n\n[workspace.package]\nversion = \"0.1.0\"\n"
        );
    }

    /// The form this command cannot edit says so rather than mangling it.
    #[test]
    fn a_multi_line_member_list_is_refused() {
        let error = with_member("[workspace]\nmembers = [\n  \"alpha\",\n]\n", "beta").unwrap_err();
        assert!(error.contains("by hand"), "{error}");
    }

    /// The one form this command cannot edit says so instead of guessing.
    #[test]
    fn a_dependency_written_as_a_table_is_refused() {
        let error = with_dependency(
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies.geometry]\nversion = \"^1\"\n",
            "geometry",
            Some("\"^2\""),
        )
        .unwrap_err();
        assert!(error.contains("by hand"), "{error}");
    }

    /// A bare requirement is the whole entry, and a source turns it into a
    /// table with the requirement kept.
    #[test]
    fn what_add_writes_depends_on_what_it_was_given() {
        assert_eq!(Added::default().entry(Some("^1.2")).unwrap(), "\"^1.2\"");
        assert_eq!(
            Added {
                registry: Some("internal".to_owned()),
                ..Added::default()
            }
            .entry(Some("^1.2"))
            .unwrap(),
            "{ version = \"^1.2\", registry = \"internal\" }"
        );
        assert_eq!(
            Added {
                git: Some("https://example.invalid/x.git".to_owned()),
                tag: Some("v1".to_owned()),
                ..Added::default()
            }
            .entry(None)
            .unwrap(),
            "{ git = \"https://example.invalid/x.git\", tag = \"v1\" }"
        );
        assert!(Added::default().entry(None).is_err());
        assert!(Added {
            git: Some("https://example.invalid/x.git".to_owned()),
            registry: Some("internal".to_owned()),
            ..Added::default()
        }
        .entry(None)
        .is_err());
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
        create_project("format-test", Some(root.clone()), false).unwrap();
        let source_path = root.join("src/main.slp");
        let unformatted = "(fn main () -> i32   ; keep\n 0)";
        fs::write(&source_path, unformatted).unwrap();
        let project = load_project(Some(root.join("Slopium.toml"))).unwrap();

        assert_eq!(
            format_project(&project, true).unwrap(),
            vec![source_path.clone()]
        );
        assert_eq!(fs::read_to_string(&source_path).unwrap(), unformatted);
        assert_eq!(
            format_project(&project, false).unwrap(),
            Vec::<PathBuf>::new()
        );
        assert!(format_project(&project, true).unwrap().is_empty());
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
        create_project("cache-test", Some(root.clone()), false).unwrap();
        let project = load_project(Some(root.join("Slopium.toml"))).unwrap();
        let source_root = project.source_root().unwrap();
        let compiler = root.join("Slopium.toml");
        let runtimes = vec![root.join("src/main.slp")];
        let inputs = CacheInputs {
            project: &project,
            source_root: &source_root,
            dependencies: &Dependencies::default(),
            target: DEFAULT_TARGET,
            profile_name: "dev",
            profile: project.manifest.profile.get("dev"),
            test: false,
            compiler: &compiler,
            runtimes: &runtimes,
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

    /// A dependency reached through two different packages is resolved once,
    /// under its own name.
    ///
    /// Before `D-035` this produced `left:shared` and `right:shared` — two
    /// namespaces, two copies in the binary — and before the walker was fixed
    /// it produced only one of them, leaving the other branch unresolvable.
    #[test]
    fn diamond_dependency_is_resolved_once_under_its_package_name() {
        let root =
            std::env::temp_dir().join(format!("slopium-diamond-test-{}", std::process::id()));
        if root.exists() {
            fs::remove_dir_all(&root).unwrap();
        }
        let write = |package: &str, dependencies: &str| {
            let directory = root.join(package);
            fs::create_dir_all(directory.join("src")).unwrap();
            fs::write(
                directory.join("Slopium.toml"),
                format!(
                    "[package]\nname = \"{package}\"\nversion = \"0.1.0\"\nsource = \"src\"\nentry = \"src/lib.slp\"\n\n[dependencies]\n{dependencies}"
                ),
            )
            .unwrap();
            fs::write(directory.join("src/lib.slp"), "(fn unused () -> i32 0)\n").unwrap();
        };

        write("shared", "");
        write("left", "shared = { path = \"../shared\" }\n");
        write("right", "shared = { path = \"../shared\" }\n");
        let application = root.join("application");
        fs::create_dir_all(application.join("src")).unwrap();
        fs::write(
            application.join("Slopium.toml"),
            "[package]\nname = \"application\"\nversion = \"0.1.0\"\nsource = \"src\"\nentry = \"src/main.slp\"\n\n[dependencies]\nleft = { path = \"../left\" }\nright = { path = \"../right\" }\n",
        )
        .unwrap();
        fs::write(application.join("src/main.slp"), "(fn main () -> i32 0)\n").unwrap();

        let manifest = application.join("Slopium.toml");
        let session = Session::open(Some(manifest.clone()), ResolveArgs::default()).unwrap();
        let project = session.workspace.member("application").unwrap();
        let namespaces = session
            .dependencies(project, "x86_64-unknown-linux-gnu")
            .unwrap()
            .packages
            .into_iter()
            .map(|dependency| dependency.namespace)
            .collect::<Vec<_>>();

        assert_eq!(namespaces, vec!["left", "right", "shared"]);

        // Resolution wrote a lockfile, and re-resolving under `--locked` is a
        // no-op rather than a rewrite.
        let lock = application.join("Slopium.lock");
        assert!(lock.is_file());
        let locked = ResolveArgs {
            locked: true,
            ..ResolveArgs::default()
        };
        Session::open(Some(manifest.clone()), locked).unwrap();

        // A lock this toolchain understands and that records nothing: out of
        // date, rather than unreadable.
        fs::write(
            &lock,
            format!("version = {}\n", slopium_manifest::lock::LOCK_FORMAT),
        )
        .unwrap();
        let error = Session::open(Some(manifest), locked).unwrap_err();
        assert!(error.contains("out of date"), "{error}");

        fs::remove_dir_all(root).unwrap();
    }

    /// `D-059`: what is about to be signed has to survive being unpacked and
    /// packed again, because a signature says these bytes *are* the package.
    #[test]
    fn only_an_archive_that_reproduces_itself_is_published() {
        let entries = vec![slopium_manifest::archive::Entry::file(
            "demo-0.1.0/Slopium.toml",
            MANIFEST.as_bytes(),
        )];
        let bytes = slopium_manifest::archive::write(&entries).unwrap();
        let digest = slopium_manifest::sha256::sha256(&bytes);
        round_trips(&bytes, &digest).expect("what `package` writes round-trips");

        // A tar whose header says one length and whose body is another is a
        // tar that unpacks to something else than it was written from.
        let mut damaged = bytes.clone();
        let last = damaged.len() - 1;
        damaged[last] ^= 0xff;
        assert!(round_trips(&damaged, &digest).is_err());
    }
}
