//! Turning a root manifest into a resolved package graph (`D-035`).
//!
//! The walker this replaces gave a dependency the namespace of the path it was
//! reached by, so one package reached two ways was two packages with two
//! namespaces and two copies in the binary. A resolved graph holds each package
//! once, keyed by name and version, and its namespace is its package name.
//!
//! Selection is maximal (`D-036`). Every source available in v0.4.0 offers
//! exactly one version of a package, so the interesting half of that rule —
//! choosing among candidates — has nothing to choose from yet and arrives with
//! the registry. What exists now is the half that already bites: collecting
//! every requirement on a name and reporting who disagreed when none is
//! satisfied.

use crate::archive::prefix_for;
use crate::manifest::{validate_package_name, Project, MANIFEST_FILE};
use crate::sha256::Digest;
use crate::source::{SourceId, SourceSpec};
use crate::sources::Sources;
use crate::std_library::{std_archive, std_language_items, STD_PACKAGE};
use crate::store::verify_tree;
use crate::version::{Version, VersionReq};
use crate::workspace::{load_project, Workspace};
use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;

/// A package, identified the way the lockfile identifies it.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PackageId {
    pub name: String,
    pub version: Version,
    pub source: SourceId,
}

impl std::fmt::Display for PackageId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{} v{}", self.name, self.version)
    }
}

/// One package in a resolved graph.
#[derive(Clone, Debug)]
pub struct ResolvedPackage {
    pub id: PackageId,
    /// Absent for the bundled library, unless a vendored copy of it replaced
    /// the bundled one — then this is the copy on disk.
    pub project: Option<Project>,
    /// Names of this package's direct dependencies, sorted.
    pub dependencies: Vec<String>,
    /// What this package's archive hashes to, for a source whose bytes cannot
    /// change underneath the lock. A path dependency is a working tree and has
    /// none: hashing one would rewrite the lock on every keystroke.
    pub checksum: Option<Digest>,
}

impl ResolvedPackage {
    /// The namespace this package's modules take. `D-035`: the package name,
    /// not the alias any particular dependent used.
    pub fn namespace(&self) -> &str {
        &self.id.name
    }
}

/// A resolved graph.
#[derive(Clone, Debug)]
pub struct Resolution {
    pub root: PackageId,
    /// Every package including the root, keyed by name — which is unique by
    /// construction, because two versions of one name is an error (`D-036`).
    pub packages: BTreeMap<String, ResolvedPackage>,
    /// Language items contributed by the root's direct dependencies.
    pub language_items: Vec<(String, String)>,
}

impl Resolution {
    /// Every package except the root, in namespace order.
    pub fn dependencies(&self) -> Vec<&ResolvedPackage> {
        self.packages
            .values()
            .filter(|package| package.id != self.root)
            .collect()
    }
}

/// Every member of a workspace resolved, and the one graph they agree on.
///
/// Members are resolved separately because each one compiles against its own
/// dependencies, but they share a lockfile — so the union has to be a function,
/// not a relation: one name, one version, whichever member reached it.
#[derive(Clone, Debug)]
pub struct WorkspaceResolution {
    pub members: BTreeMap<String, Resolution>,
    /// Every package any member reaches, unique by name. This is what the lock
    /// records and what "the shared dependency appears once" means.
    pub packages: BTreeMap<String, ResolvedPackage>,
}

impl WorkspaceResolution {
    pub fn member(&self, name: &str) -> Result<&Resolution, String> {
        self.members
            .get(name)
            .ok_or_else(|| format!("package `{name}` was not resolved"))
    }
}

/// Resolve every member of a workspace.
pub fn resolve_workspace(
    workspace: &Workspace,
    toolchain_version: &Version,
    sources: &Sources,
) -> Result<WorkspaceResolution, String> {
    let mut members = BTreeMap::new();
    let mut packages: BTreeMap<String, ResolvedPackage> = BTreeMap::new();
    let mut reached_by: BTreeMap<String, String> = BTreeMap::new();

    for (name, project) in &workspace.members {
        let resolution = resolve(project, workspace, toolchain_version, sources)?;
        for package in resolution.packages.values() {
            match packages.get(&package.id.name) {
                Some(existing) if existing.id == package.id => {}
                Some(existing) if existing.id.version == package.id.version => {
                    let first = &reached_by[&package.id.name];
                    return Err(format!(
                        "`{}` is required from two sources in one workspace: `{}` through `{first}` and `{}` through `{name}`. One lockfile cannot record both",
                        package.id.name, existing.id.source, package.id.source
                    ));
                }
                Some(existing) => {
                    let first = &reached_by[&package.id.name];
                    return Err(format!(
                        "`{}` is required at two versions in one workspace: {} through `{first}` and {} through `{name}`. One lockfile cannot record both",
                        package.id.name, existing.id.version, package.id.version
                    ));
                }
                None => {
                    reached_by.insert(package.id.name.clone(), name.clone());
                    packages.insert(package.id.name.clone(), package.clone());
                }
            }
        }
        members.insert(name.clone(), resolution);
    }

    Ok(WorkspaceResolution { members, packages })
}

/// Who asked for what, kept so a conflict can name both sides.
#[derive(Clone, Debug)]
struct Requirement {
    requested_by: String,
    request: VersionReq,
}

/// Resolve `root` and everything it reaches.
///
/// `toolchain_version` is the compiler's own version, which is what the bundled
/// library is versioned as — it ships with the compiler and cannot skew from it.
pub fn resolve(
    root: &Project,
    workspace: &Workspace,
    toolchain_version: &Version,
    sources: &Sources,
) -> Result<Resolution, String> {
    let mut packages: BTreeMap<String, ResolvedPackage> = BTreeMap::new();
    let mut requirements: BTreeMap<String, Vec<Requirement>> = BTreeMap::new();
    let mut language_items: Vec<(String, String)> = Vec::new();
    let mut language_item_source: Option<String> = None;

    let root_id = PackageId {
        name: root.name.clone(),
        version: root.version.clone(),
        source: SourceId::Path(root.root.clone()),
    };
    packages.insert(
        root_id.name.clone(),
        ResolvedPackage {
            id: root_id.clone(),
            project: Some(root.clone()),
            dependencies: root.dependencies.keys().cloned().collect(),
            checksum: None,
        },
    );

    // Each package is queued with the source it came from, because what a
    // `path` dependency is allowed to mean depends on it (`D-051`).
    let mut queue = VecDeque::new();
    queue.push_back((root_id.source.clone(), root.clone()));
    let mut is_root = true;

    while let Some((dependent_source, project)) = queue.pop_front() {
        let dependent = project.name.clone();
        for (declared, spec) in &project.dependencies {
            validate_package_name(declared)?;
            requirements
                .entry(declared.clone())
                .or_default()
                .push(Requirement {
                    requested_by: dependent.clone(),
                    request: spec.requirement(),
                });

            let (id, dependency, checksum) = match spec.source(declared)? {
                SourceSpec::Toolchain => {
                    if declared != STD_PACKAGE {
                        return Err(format!(
                            "dependency `{declared}` cannot use the toolchain source; the bundled package is named `{STD_PACKAGE}`"
                        ));
                    }
                    let (_, digest) = std_archive(toolchain_version)?;
                    let id = PackageId {
                        name: STD_PACKAGE.to_owned(),
                        version: toolchain_version.clone(),
                        source: SourceId::Toolchain,
                    };
                    let vendored = replacement(workspace, &id, &digest)?;
                    (id, vendored, Some(digest))
                }
                SourceSpec::Git { url, reference } => {
                    let pin = sources.pin_git(declared, &url, &reference)?;
                    let id = PackageId {
                        name: declared.clone(),
                        version: pin.version.clone(),
                        source: pin.source.clone(),
                    };
                    // A vendored copy is checked and used without the store
                    // being touched at all, which is what makes `vendor`
                    // followed by `--offline` work on a machine with no `git`.
                    let dependency = match replacement(workspace, &id, &pin.checksum)? {
                        Some(project) => project,
                        None => {
                            let root = sources.checkout(&id, &pin.checksum)?;
                            load_project(Some(root.join(MANIFEST_FILE)))?
                        }
                    };
                    if dependency.name != id.name || dependency.version != id.version {
                        return Err(format!(
                            "`{url}` at {} is `{} v{}`, but it was resolved as `{id}`",
                            pin.source, dependency.name, dependency.version
                        ));
                    }
                    (id, Some(dependency), Some(pin.checksum))
                }
                SourceSpec::Path(relative) => {
                    // `D-051`: a git package is unpacked into the store, so a
                    // relative path from one either escapes the package or
                    // names a directory whose absolute path a lock must not
                    // record. Both have answers; neither is in this release.
                    if matches!(dependent_source, SourceId::Git { .. }) {
                        return Err(format!(
                            "`{dependent}` comes from git and declares the `path` dependency `{declared}`; a package fetched from a repository cannot have one yet, because there is no way to write where it lives into a lockfile that another machine could read"
                        ));
                    }
                    let dependency = load_path_dependency(&project, workspace, &relative)?;
                    // `D-035`: the key in `[dependencies]` *is* the package
                    // name, because the name is what the namespace and the lock
                    // are built from. An alias that differed from it would give
                    // one package two names.
                    if dependency.name != *declared {
                        return Err(format!(
                            "`{dependent}` declares dependency `{declared}`, but the package at `{}` is named `{}`; the key in `[dependencies]` must be the package name",
                            dependency.root.display(),
                            dependency.name
                        ));
                    }
                    (
                        PackageId {
                            name: dependency.name.clone(),
                            version: dependency.version.clone(),
                            source: SourceId::Path(dependency.root.clone()),
                        },
                        Some(dependency),
                        None,
                    )
                }
            };

            if is_root {
                collect_language_items(
                    declared,
                    dependency.as_ref(),
                    &mut language_items,
                    &mut language_item_source,
                )?;
            }

            match packages.get(&id.name) {
                Some(existing) if existing.id == id => continue,
                // `D-038`, restated for git by `D-049`: two dependents that
                // disagree about which commit of a repository to take is a
                // question with two answers, and picking one silently is worse
                // than reporting it.
                Some(existing) if existing.id.version == id.version => {
                    return Err(format!(
                        "`{}` is required from two sources: `{}` and `{}`. A package name resolves from one source in a graph",
                        id.name, existing.id.source, id.source
                    ));
                }
                Some(existing) => {
                    return Err(format!(
                        "`{}` is required at two versions: {} and {}. Two incompatible versions of one package cannot coexist in a graph",
                        id.name, existing.id.version, id.version
                    ));
                }
                None => {}
            }

            let source = id.source.clone();

            packages.insert(
                id.name.clone(),
                ResolvedPackage {
                    id,
                    dependencies: dependency
                        .as_ref()
                        .map(|project| project.dependencies.keys().cloned().collect())
                        .unwrap_or_default(),
                    project: dependency.clone(),
                    checksum,
                },
            );
            if let Some(dependency) = dependency {
                queue.push_back((source, dependency));
            }
        }
        is_root = false;
    }

    check_requirements(&packages, &requirements)?;
    reject_cycles(&packages, &root_id.name)?;

    Ok(Resolution {
        root: root_id,
        packages,
        language_items,
    })
}

/// A vendored copy standing in for a package, if one is configured.
///
/// Replacement is a property of the checkout, not of the graph: the package
/// keeps its identity, its source and its lock entry, and only the bytes the
/// compiler is handed come from somewhere else (`D-047`). That is what makes
/// `slopium vendor` safe to run — it cannot change what a build resolves to,
/// only where the same thing is read from.
fn replacement(
    workspace: &Workspace,
    id: &PackageId,
    checksum: &Digest,
) -> Result<Option<Project>, String> {
    let source = id.source.config_name();
    let Some(directory) = workspace.config.replacement(source, &workspace.root)? else {
        return Ok(None);
    };
    let root = directory.join(&id.name);
    if !root.is_dir() {
        return Err(format!(
            "`{source}` is replaced by the vendored packages in `{}`, but `{}` is not there; run `slopium vendor`",
            directory.display(),
            id.name
        ));
    }
    verify_tree(
        &root,
        &prefix_for(&id.name, &id.version),
        checksum,
        &id.to_string(),
    )?;
    let project = load_project(Some(root.join(MANIFEST_FILE)))?;
    if project.name != id.name || project.version != id.version {
        return Err(format!(
            "the vendored copy at `{}` is `{} v{}`, but it stands in for `{id}`",
            root.display(),
            project.name,
            project.version
        ));
    }
    Ok(Some(project))
}

/// Load a `path` dependency.
///
/// A path that lands on a workspace member resolves to that member rather than
/// being read again as a stranger: a member's manifest may inherit fields from
/// the workspace, and re-reading it from here would either miss them or have to
/// rediscover the workspace to find them.
fn load_path_dependency(
    dependent: &Project,
    workspace: &Workspace,
    relative: &PathBuf,
) -> Result<Project, String> {
    let root = dependent.root.join(relative);
    if let Ok(canonical) = root.canonicalize() {
        if let Some(member) = workspace.member_at(&canonical) {
            return Ok(member.clone());
        }
    }
    let manifest = if root.is_dir() {
        root.join(MANIFEST_FILE)
    } else {
        root
    };
    load_project(Some(manifest))
}

/// Language items come from whichever direct dependency declares them.
///
/// This used to key off the alias `std`, so a replacement library had to be
/// *called* `std` to be believed. `D-011` says the standard library is an
/// ordinary dependency; what makes it the standard library is that it declares
/// the language items, not what it is named.
fn collect_language_items(
    declared: &str,
    dependency: Option<&Project>,
    items: &mut Vec<(String, String)>,
    source: &mut Option<String>,
) -> Result<(), String> {
    let contributed = match dependency {
        None => std_language_items(),
        Some(project) if !project.manifest.language_items.is_empty() => project
            .manifest
            .language_items
            .entries()
            .into_iter()
            .map(|(name, path)| (name, format!("{}:{path}", project.name)))
            .collect(),
        Some(_) => return Ok(()),
    };
    if let Some(previous) = source {
        return Err(format!(
            "`{previous}` and `{declared}` both define `[language-items]`; a package graph has one standard library"
        ));
    }
    *source = Some(declared.to_owned());
    *items = contributed;
    Ok(())
}

/// Every requirement recorded on a name must accept the version selected for it.
fn check_requirements(
    packages: &BTreeMap<String, ResolvedPackage>,
    requirements: &BTreeMap<String, Vec<Requirement>>,
) -> Result<(), String> {
    for (name, requests) in requirements {
        let Some(package) = packages.get(name) else {
            continue;
        };
        let unsatisfied = requests
            .iter()
            .filter(|requirement| !requirement.request.matches(&package.id.version))
            .collect::<Vec<_>>();
        if unsatisfied.is_empty() {
            continue;
        }
        let complaints = unsatisfied
            .iter()
            .map(|requirement| {
                format!(
                    "`{}` requires {}",
                    requirement.requested_by, requirement.request
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "cannot select a version of `{name}`: {complaints}, but the only candidate is {}",
            package.id.version
        ));
    }
    Ok(())
}

/// Reject cycles over the resolved graph.
///
/// Deduplication means a repeated name is no longer proof of a cycle — a
/// diamond repeats a name legitimately — so this is a colored depth-first
/// search over the resolved edges rather than a check against the walk stack.
fn reject_cycles(packages: &BTreeMap<String, ResolvedPackage>, root: &str) -> Result<(), String> {
    #[derive(Clone, Copy, PartialEq)]
    enum State {
        Unvisited,
        InProgress,
        Done,
    }

    fn visit(
        name: &str,
        packages: &BTreeMap<String, ResolvedPackage>,
        state: &mut BTreeMap<String, State>,
        path: &mut Vec<String>,
    ) -> Result<(), String> {
        match state.get(name).copied().unwrap_or(State::Unvisited) {
            State::Done => return Ok(()),
            State::InProgress => {
                let start = path.iter().position(|entry| entry == name).unwrap_or(0);
                let mut cycle = path[start..].to_vec();
                cycle.push(name.to_owned());
                return Err(format!("package dependency cycle: {}", cycle.join(" -> ")));
            }
            State::Unvisited => {}
        }
        state.insert(name.to_owned(), State::InProgress);
        path.push(name.to_owned());
        if let Some(package) = packages.get(name) {
            for dependency in &package.dependencies {
                visit(dependency, packages, state, path)?;
            }
        }
        path.pop();
        state.insert(name.to_owned(), State::Done);
        Ok(())
    }

    let mut state = BTreeMap::new();
    let mut path = Vec::new();
    visit(root, packages, &mut state, &mut path)?;
    // A package unreachable from the root cannot exist here, but visiting the
    // rest keeps the check total if that ever changes.
    for name in packages.keys() {
        visit(name, packages, &mut state, &mut path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Workspace {
        root: PathBuf,
    }

    impl Workspace {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "slopium-resolve-{label}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            if root.exists() {
                fs::remove_dir_all(&root).unwrap();
            }
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn package(&self, name: &str, version: &str, dependencies: &str) -> PathBuf {
            let directory = self.root.join(name);
            fs::create_dir_all(directory.join("src")).unwrap();
            fs::write(
                directory.join(MANIFEST_FILE),
                format!(
                    "[package]\nname = \"{name}\"\nversion = \"{version}\"\nsource = \"src\"\nentry = \"src/lib.slp\"\n\n[dependencies]\n{dependencies}"
                ),
            )
            .unwrap();
            fs::write(directory.join("src/lib.slp"), "(fn unused () -> i32 0)\n").unwrap();
            directory
        }

        fn resolve(&self, package: &str) -> Result<Resolution, String> {
            let manifest = self.root.join(package).join(MANIFEST_FILE);
            let workspace = crate::workspace::load_workspace(Some(manifest))?;
            let project = workspace.select(None, false)?[0].clone();
            super::resolve(
                &project,
                &workspace,
                &Version::new(0, 3, 7),
                &self.sources(),
            )
        }

        /// A store inside the scratch directory, so a test never writes to the
        /// developer's own — and never has to be told to clean it up.
        fn sources(&self) -> Sources {
            Sources::new(
                crate::store::Store::at(self.root.join(".store")),
                crate::store::Access::Online,
                false,
            )
        }

        fn write_manifest(&self, relative: &str, contents: &str) {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }
    }

    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn names(resolution: &Resolution) -> Vec<String> {
        resolution
            .dependencies()
            .into_iter()
            .map(|package| package.namespace().to_owned())
            .collect()
    }

    /// The toolchain's bytes do not change under the lock, so they are the
    /// first thing a lockfile has a checksum for.
    #[test]
    fn the_bundled_library_is_locked_by_its_digest() {
        let workspace = Workspace::new("toolchain-checksum");
        workspace.package("application", "1.0.0", "std = { toolchain = true }\n");
        let resolution = workspace.resolve("application").unwrap();
        let (_, digest) = std_archive(&Version::new(0, 3, 7)).unwrap();
        assert_eq!(resolution.packages["std"].checksum, Some(digest));
        assert_eq!(resolution.packages["application"].checksum, None);
    }

    /// Vendoring may change where bytes are read from and nothing else: the
    /// package keeps its source, its identity and its lock entry (`D-047`).
    #[test]
    fn a_replaced_source_is_read_from_the_vendor_directory() {
        let workspace = Workspace::new("replacement");
        workspace.package("application", "1.0.0", "std = { toolchain = true }\n");
        let version = Version::new(0, 3, 7);
        let entries = crate::std_library::std_entries(&version);
        crate::store::unpack(&entries, &workspace.root.join("application/vendor/std")).unwrap();
        workspace.write_manifest(
            "application/.slopium/config.toml",
            "[source.toolchain]\nreplace-with = \"vendored\"\n\n[source.vendored]\ndirectory = \"vendor\"\n",
        );

        let resolution = workspace.resolve("application").unwrap();
        let standard = &resolution.packages["std"];
        assert_eq!(standard.id.source, SourceId::Toolchain);
        assert_eq!(standard.checksum, Some(std_archive(&version).unwrap().1));
        assert_eq!(
            standard
                .project
                .as_ref()
                .map(|project| project.root.clone()),
            Some(workspace.root.join("application/vendor/std"))
        );
        assert_eq!(
            resolution.language_items,
            crate::std_library::std_language_items()
        );

        // The whole point of the checksum: an edited copy is not the package.
        fs::write(
            workspace.root.join("application/vendor/std/src/option.slp"),
            "(export Option)\n",
        )
        .unwrap();
        let error = workspace.resolve("application").unwrap_err();
        assert!(error.contains("SL1012"), "{error}");
    }

    /// Build a repository holding one package, and return its URL and the
    /// commit on its default branch.
    ///
    /// In a temporary directory at test time, because `nix flake check` runs
    /// this suite in a sandbox with no network — a fixture that had to be
    /// cloned would be a test that cannot run where it matters.
    fn repository(workspace: &Workspace, name: &str, version: &str) -> (String, String) {
        let root = workspace.root.join(format!("{name}.git"));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join(MANIFEST_FILE),
            format!("[package]\nname = \"{name}\"\nversion = \"{version}\"\nsource = \"src\"\n"),
        )
        .unwrap();
        fs::write(root.join("src/lib.slp"), "(fn unused () -> i32 0)\n").unwrap();

        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .current_dir(&root)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        };
        git(&["init", "--quiet", "--initial-branch=main"]);
        git(&["config", "user.name", "Test"]);
        git(&["config", "user.email", "test@example.invalid"]);
        git(&["add", "--all"]);
        git(&["commit", "--quiet", "--message", "first"]);
        (root.display().to_string(), git(&["rev-parse", "HEAD"]))
    }

    /// The whole of v0.4.3 in one assertion: a repository becomes a package
    /// pinned to a commit and to the digest of its archive, and the compiler is
    /// handed a directory in the store.
    #[test]
    fn a_git_dependency_is_pinned_to_a_commit_and_a_digest() {
        let workspace = Workspace::new("git");
        let (url, commit) = repository(&workspace, "geometry", "1.4.0");
        workspace.package(
            "application",
            "1.0.0",
            &format!("geometry = {{ git = \"{url}\" }}\n"),
        );

        let resolution = workspace.resolve("application").unwrap();
        let geometry = &resolution.packages["geometry"];
        assert_eq!(geometry.id.version, Version::new(1, 4, 0));
        assert_eq!(
            geometry.id.source,
            SourceId::Git {
                url: url.clone(),
                reference: crate::source::GitReference::DefaultBranch,
                rev: commit.clone(),
            }
        );
        let checksum = geometry
            .checksum
            .expect("a git package is pinned by digest");
        let store = crate::store::Store::at(workspace.root.join(".store"));
        assert!(store.holds(&checksum), "the archive was not stored");
        assert_eq!(
            geometry
                .project
                .as_ref()
                .map(|project| project.root.clone()),
            Some(store.checkout_path(&checksum))
        );
        // Resolving again is the same answer, and the second time the digest is
        // re-derived from bytes that were already there.
        assert_eq!(
            workspace.resolve("application").unwrap().packages["geometry"].checksum,
            Some(checksum)
        );
    }

    /// `D-051`: a package unpacked into the store cannot have a `path`
    /// dependency, because there is no portable way to write one down.
    #[test]
    fn a_git_package_may_not_declare_a_path_dependency() {
        let workspace = Workspace::new("git-path");
        let (url, _) = repository(&workspace, "geometry", "1.4.0");
        fs::write(
            workspace.root.join("geometry.git").join(MANIFEST_FILE),
            "[package]\nname = \"geometry\"\nversion = \"1.4.0\"\nsource = \"src\"\n\n[dependencies]\nhelper = { path = \"../helper\" }\n",
        )
        .unwrap();
        for args in [
            vec!["add", "--all"],
            vec!["commit", "--quiet", "--message", "second"],
        ] {
            std::process::Command::new("git")
                .current_dir(workspace.root.join("geometry.git"))
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .args(&args)
                .output()
                .unwrap();
        }
        workspace.package(
            "application",
            "1.0.0",
            &format!("geometry = {{ git = \"{url}\" }}\n"),
        );

        let error = workspace.resolve("application").unwrap_err();
        assert!(error.contains("comes from git"), "{error}");
        assert!(error.contains("`helper`"), "{error}");
    }

    /// The defect that motivated `D-035`, from the other direction: a package
    /// reached through two others appears once, not twice.
    #[test]
    fn a_diamond_holds_one_copy_of_the_shared_package() {
        let workspace = Workspace::new("diamond");
        workspace.package("shared", "1.0.0", "");
        workspace.package("left", "1.0.0", "shared = { path = \"../shared\" }\n");
        workspace.package("right", "1.0.0", "shared = { path = \"../shared\" }\n");
        workspace.package(
            "application",
            "1.0.0",
            "left = { path = \"../left\" }\nright = { path = \"../right\" }\n",
        );

        let resolution = workspace.resolve("application").unwrap();
        assert_eq!(names(&resolution), vec!["left", "right", "shared"]);
        assert_eq!(resolution.packages.len(), 4);
    }

    #[test]
    fn a_transitive_chain_is_flattened_to_package_names() {
        let workspace = Workspace::new("chain");
        workspace.package("foundation", "1.0.0", "");
        workspace.package(
            "mathlib",
            "1.0.0",
            "foundation = { path = \"../foundation\" }\n",
        );
        workspace.package(
            "application",
            "1.0.0",
            "mathlib = { path = \"../mathlib\" }\n",
        );

        let resolution = workspace.resolve("application").unwrap();
        assert_eq!(names(&resolution), vec!["foundation", "mathlib"]);
    }

    #[test]
    fn two_versions_of_one_name_are_rejected() {
        let workspace = Workspace::new("two-versions");
        fs::create_dir_all(workspace.root.join("old")).unwrap();
        workspace.package("shared", "2.0.0", "");
        // A second package that calls itself `shared` at another version.
        let old = workspace.root.join("old-shared");
        fs::create_dir_all(old.join("src")).unwrap();
        fs::write(
            old.join(MANIFEST_FILE),
            "[package]\nname = \"shared\"\nversion = \"1.0.0\"\nsource = \"src\"\nentry = \"src/lib.slp\"\n",
        )
        .unwrap();
        fs::write(old.join("src/lib.slp"), "(fn unused () -> i32 0)\n").unwrap();

        workspace.package("left", "1.0.0", "shared = { path = \"../shared\" }\n");
        workspace.package("right", "1.0.0", "shared = { path = \"../old-shared\" }\n");
        workspace.package(
            "application",
            "1.0.0",
            "left = { path = \"../left\" }\nright = { path = \"../right\" }\n",
        );

        let error = workspace.resolve("application").unwrap_err();
        assert!(error.contains("two versions"), "{error}");
        assert!(error.contains("shared"), "{error}");
    }

    #[test]
    fn a_requirement_the_candidate_cannot_meet_names_who_asked() {
        let workspace = Workspace::new("conflict");
        workspace.package("shared", "1.0.0", "");
        workspace.package(
            "application",
            "1.0.0",
            "shared = { path = \"../shared\", version = \"^2.0.0\" }\n",
        );

        let error = workspace.resolve("application").unwrap_err();
        assert!(
            error.contains("cannot select a version of `shared`"),
            "{error}"
        );
        assert!(error.contains("`application` requires ^2.0.0"), "{error}");
        assert!(error.contains("1.0.0"), "{error}");
    }

    #[test]
    fn a_satisfied_requirement_resolves() {
        let workspace = Workspace::new("satisfied");
        workspace.package("shared", "1.4.0", "");
        workspace.package(
            "application",
            "1.0.0",
            "shared = { path = \"../shared\", version = \"^1.2\" }\n",
        );
        assert_eq!(
            names(&workspace.resolve("application").unwrap()),
            vec!["shared"]
        );
    }

    #[test]
    fn a_cycle_is_rejected() {
        let workspace = Workspace::new("cycle");
        workspace.package("a", "1.0.0", "b = { path = \"../b\" }\n");
        workspace.package("b", "1.0.0", "a = { path = \"../a\" }\n");

        let error = workspace.resolve("a").unwrap_err();
        assert!(error.contains("package dependency cycle"), "{error}");
    }

    #[test]
    fn a_key_that_is_not_the_package_name_is_rejected() {
        let workspace = Workspace::new("alias");
        workspace.package("mathlib", "1.0.0", "");
        workspace.package("application", "1.0.0", "math = { path = \"../mathlib\" }\n");

        let error = workspace.resolve("application").unwrap_err();
        assert!(error.contains("must be the package name"), "{error}");
    }

    #[test]
    fn the_toolchain_supplies_language_items() {
        let workspace = Workspace::new("toolchain");
        workspace.package("application", "1.0.0", "std = { toolchain = true }\n");

        let resolution = workspace.resolve("application").unwrap();
        assert_eq!(names(&resolution), vec!["std"]);
        assert!(resolution
            .language_items
            .contains(&("option".to_owned(), "std:option:Option".to_owned())));
    }

    /// `D-011` says the standard library is an ordinary dependency. What makes
    /// one the standard library is `[language-items]`, not the name `std`.
    #[test]
    fn any_dependency_declaring_language_items_supplies_them() {
        let workspace = Workspace::new("custom-std");
        let custom = workspace.package("custom-std", "1.0.0", "");
        fs::write(
            custom.join(MANIFEST_FILE),
            "[package]\nname = \"custom-std\"\nversion = \"1.0.0\"\nsource = \"src\"\nentry = \"src/lib.slp\"\n\n[language-items]\noption = \"option:Option\"\n",
        )
        .unwrap();
        workspace.package(
            "application",
            "1.0.0",
            "custom-std = { path = \"../custom-std\" }\n",
        );

        let resolution = workspace.resolve("application").unwrap();
        assert_eq!(
            resolution.language_items,
            vec![("option".to_owned(), "custom-std:option:Option".to_owned())]
        );
    }

    #[test]
    fn two_standard_libraries_are_rejected() {
        let workspace = Workspace::new("two-stds");
        let custom = workspace.package("custom-std", "1.0.0", "");
        fs::write(
            custom.join(MANIFEST_FILE),
            "[package]\nname = \"custom-std\"\nversion = \"1.0.0\"\nsource = \"src\"\nentry = \"src/lib.slp\"\n\n[language-items]\noption = \"option:Option\"\n",
        )
        .unwrap();
        workspace.package(
            "application",
            "1.0.0",
            "custom-std = { path = \"../custom-std\" }\nstd = { toolchain = true }\n",
        );

        let error = workspace.resolve("application").unwrap_err();
        assert!(error.contains("one standard library"), "{error}");
    }

    #[test]
    fn resolution_is_deterministic() {
        let workspace = Workspace::new("determinism");
        workspace.package("shared", "1.0.0", "");
        workspace.package("left", "1.0.0", "shared = { path = \"../shared\" }\n");
        workspace.package("right", "1.0.0", "shared = { path = \"../shared\" }\n");
        workspace.package(
            "application",
            "1.0.0",
            "right = { path = \"../right\" }\nleft = { path = \"../left\" }\n",
        );

        let first = names(&workspace.resolve("application").unwrap());
        for _ in 0..8 {
            assert_eq!(names(&workspace.resolve("application").unwrap()), first);
        }
        assert_eq!(first, vec!["left", "right", "shared"]);
    }

    /// A member reached as a path dependency is the member, not a second
    /// reading of it — which is what lets a member inherit its version.
    #[test]
    fn a_path_dependency_on_a_member_resolves_to_the_member() {
        let workspace = Workspace::new("members");
        workspace.write_manifest(
            MANIFEST_FILE,
            "[workspace]\nmembers = [\"application\", \"helper\"]\n\n[workspace.package]\nversion = \"3.1.4\"\n",
        );
        workspace.package("helper", "1.0.0", "");
        workspace.write_manifest(
            "helper/Slopium.toml",
            "[package]\nname = \"helper\"\nversion.workspace = true\nsource = \"src\"\nentry = \"src/lib.slp\"\n",
        );
        workspace.package(
            "application",
            "1.0.0",
            "helper = { path = \"../helper\" }\n",
        );

        let loaded =
            crate::workspace::load_workspace(Some(workspace.root.join(MANIFEST_FILE))).unwrap();
        let resolved =
            resolve_workspace(&loaded, &Version::new(0, 3, 7), &workspace.sources()).unwrap();
        assert_eq!(
            resolved.packages["helper"].id.version,
            Version::new(3, 1, 4)
        );
        // Two members, one of them a dependency of the other: three entries
        // would mean the member was read twice.
        assert_eq!(resolved.packages.len(), 2);
        assert_eq!(
            names(resolved.member("application").unwrap()),
            vec!["helper"]
        );
    }

    #[test]
    fn a_missing_dependency_directory_is_reported() {
        let workspace = Workspace::new("missing");
        workspace.package(
            "application",
            "1.0.0",
            "absent = { path = \"../absent\" }\n",
        );
        let error = workspace.resolve("application").unwrap_err();
        assert!(error.contains("absent"), "{error}");
    }
}
