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

use crate::manifest::{load_project, validate_package_name, Project, MANIFEST_FILE};
use crate::source::{SourceId, SourceSpec};
use crate::std_library::{std_language_items, STD_PACKAGE};
use crate::version::{Version, VersionReq};
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
    /// Absent for the bundled library, which has no manifest on disk.
    pub project: Option<Project>,
    /// Names of this package's direct dependencies, sorted.
    pub dependencies: Vec<String>,
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
pub fn resolve(root: &Project, toolchain_version: &Version) -> Result<Resolution, String> {
    let mut packages: BTreeMap<String, ResolvedPackage> = BTreeMap::new();
    let mut requirements: BTreeMap<String, Vec<Requirement>> = BTreeMap::new();
    let mut language_items: Vec<(String, String)> = Vec::new();
    let mut language_item_source: Option<String> = None;

    let root_id = PackageId {
        name: root.name().to_owned(),
        version: root.version().clone(),
        source: SourceId::Path(root.root.clone()),
    };
    packages.insert(
        root_id.name.clone(),
        ResolvedPackage {
            id: root_id.clone(),
            project: Some(root.clone()),
            dependencies: root.manifest.dependencies.keys().cloned().collect(),
        },
    );

    let mut queue = VecDeque::new();
    queue.push_back(root.clone());
    let mut is_root = true;

    while let Some(project) = queue.pop_front() {
        let dependent = project.name().to_owned();
        for (declared, spec) in &project.manifest.dependencies {
            validate_package_name(declared)?;
            requirements
                .entry(declared.clone())
                .or_default()
                .push(Requirement {
                    requested_by: dependent.clone(),
                    request: spec.requirement(),
                });

            let (id, dependency) = match spec.source(declared)? {
                SourceSpec::Toolchain => {
                    if declared != STD_PACKAGE {
                        return Err(format!(
                            "dependency `{declared}` cannot use the toolchain source; the bundled package is named `{STD_PACKAGE}`"
                        ));
                    }
                    (
                        PackageId {
                            name: STD_PACKAGE.to_owned(),
                            version: toolchain_version.clone(),
                            source: SourceId::Toolchain,
                        },
                        None,
                    )
                }
                SourceSpec::Path(relative) => {
                    let dependency = load_path_dependency(&project, &relative)?;
                    // `D-035`: the key in `[dependencies]` *is* the package
                    // name, because the name is what the namespace and the lock
                    // are built from. An alias that differed from it would give
                    // one package two names.
                    if dependency.name() != declared {
                        return Err(format!(
                            "`{dependent}` declares dependency `{declared}`, but the package at `{}` is named `{}`; the key in `[dependencies]` must be the package name",
                            dependency.root.display(),
                            dependency.name()
                        ));
                    }
                    (
                        PackageId {
                            name: dependency.name().to_owned(),
                            version: dependency.version().clone(),
                            source: SourceId::Path(dependency.root.clone()),
                        },
                        Some(dependency),
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
                Some(existing) => {
                    return Err(format!(
                        "`{}` is required at two versions: {} and {}. Two incompatible versions of one package cannot coexist in a graph",
                        id.name, existing.id.version, id.version
                    ));
                }
                None => {}
            }

            packages.insert(
                id.name.clone(),
                ResolvedPackage {
                    id,
                    dependencies: dependency
                        .as_ref()
                        .map(|project| project.manifest.dependencies.keys().cloned().collect())
                        .unwrap_or_default(),
                    project: dependency.clone(),
                },
            );
            if let Some(dependency) = dependency {
                queue.push_back(dependency);
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

fn load_path_dependency(dependent: &Project, relative: &PathBuf) -> Result<Project, String> {
    let root = dependent.root.join(relative);
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
            .map(|(name, path)| (name, format!("{}:{path}", project.name())))
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
            let project = load_project(Some(self.root.join(package).join(MANIFEST_FILE)))?;
            super::resolve(&project, &Version::new(0, 3, 7))
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
