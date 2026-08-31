//! Turning a root manifest into a resolved package graph (`D-035`).
//!
//! The walker this replaces gave a dependency the namespace of the path it was
//! reached by, so one package reached two ways was two packages with two
//! namespaces and two copies in the binary. A resolved graph holds each package
//! once, keyed by name and version, and its namespace is its package name.
//!
//! Selection is maximal with backtracking (`D-036`). Until the registry there
//! was nothing to select *from* — every source offered exactly one version of a
//! package — so the search below has one candidate per name for a path, a
//! toolchain or a git dependency, and a list for a registry. That is the only
//! difference: a diamond that needs an older version of one dependent to make
//! its shared dependency satisfiable is resolved by trying the older one.
//!
//! What a search never does is guess. A dependency the lock pins is offered its
//! pinned version first and, if that works, the index is not read at all; a
//! registry package's requirements come from the index during the search and
//! are checked against its manifest once the graph settles (`D-055`).

use crate::archive::prefix_for;
use crate::codes;
use crate::manifest::{validate_package_name, Project, MANIFEST_FILE};
use crate::registry::{IndexEntry, IndexSource};
use crate::sha256::Digest;
use crate::source::{GitReference, SourceId, SourceSpec, DEFAULT_REGISTRY};
use crate::sources::{Pin, Sources};
use crate::std_library::{language_items_of, std_archive, toolchain_package};
use crate::store::verify_tree;
use crate::version::{Version, VersionReq};
use crate::workspace::{load_project, Workspace};
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::path::{Path, PathBuf};

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
                        "{}: `{}` is required from two sources in one workspace: `{}` through `{first}` and `{}` through `{name}`. One lockfile cannot record both",
                        codes::TWO_SOURCES,
                        package.id.name, existing.id.source, package.id.source
                    ));
                }
                Some(existing) => {
                    let first = &reached_by[&package.id.name];
                    return Err(format!(
                        "{}: `{}` is required at two versions in one workspace: {} through `{first}` and {} through `{name}`. One lockfile cannot record both",
                        codes::TWO_VERSIONS,
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

/// A source as one requirement names it: the manifest's `SourceSpec` with the
/// parts that depend on who was asking already worked out.
///
/// Comparing one of these against a `SourceId` is how the search decides
/// whether a package already chosen for a name is the one being asked for
/// (`D-038`), and it answers without fetching anything.
#[derive(Clone, Debug, Eq, PartialEq)]
enum Want {
    Path(PathBuf),
    Toolchain,
    Git {
        url: String,
        reference: GitReference,
    },
    Registry {
        index: String,
    },
}

impl Want {
    fn matches(&self, source: &SourceId) -> bool {
        match (self, source) {
            (Self::Path(wanted), SourceId::Path(chosen)) => same_directory(wanted, chosen),
            (Self::Toolchain, SourceId::Toolchain) => true,
            (
                Self::Git { url, reference },
                SourceId::Git {
                    url: chosen,
                    reference: chosen_reference,
                    ..
                },
            ) => url == chosen && reference == chosen_reference,
            (Self::Registry { index }, SourceId::Registry { index: chosen }) => index == chosen,
            _ => false,
        }
    }
}

impl fmt::Display for Want {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(path) => write!(formatter, "the directory `{}`", path.display()),
            Self::Toolchain => formatter.write_str("the toolchain"),
            Self::Git { url, reference } => write!(formatter, "`{url}` at {reference}"),
            Self::Registry { index } => write!(formatter, "the registry `{index}`"),
        }
    }
}

/// Two paths are one directory if they name one, whether or not they are
/// spelled the same — `../shared` reached from two members is one package.
fn same_directory(left: &Path, right: &Path) -> bool {
    left == right
        || match (left.canonicalize(), right.canonicalize()) {
            (Ok(left), Ok(right)) => left == right,
            _ => false,
        }
}

/// One requirement to satisfy: who wrote it, what it asks for, and where from.
#[derive(Clone, Debug)]
struct Need {
    dependent: String,
    name: String,
    requirement: VersionReq,
    want: Want,
}

/// One package chosen for a name, and what choosing it demands.
#[derive(Clone, Debug)]
struct Chosen {
    id: PackageId,
    checksum: Option<Digest>,
    /// Absent while the search is running for a package the index offered:
    /// its requirements are known without downloading it, and downloading
    /// every candidate is the cost an index exists to avoid.
    project: Option<Project>,
    /// The index entry that offered it, kept so the manifest can be checked
    /// against it once the graph settles (`D-055`).
    entry: Option<IndexEntry>,
    needs: Vec<Need>,
}

impl Chosen {
    fn dependencies(&self) -> Vec<String> {
        let mut names: Vec<String> = self.needs.iter().map(|need| need.name.clone()).collect();
        names.sort();
        names.dedup();
        names
    }
}

/// What a source has to offer for one name.
struct Options {
    /// Every version, newest first, before the requirement is applied.
    all: Vec<Chosen>,
    /// Versions the index has and will not select (`D-055`).
    yanked: Vec<Version>,
    /// Whether `all` is the lockfile's answer rather than the whole list, in
    /// which case there is somewhere to fall back to if it does not work out.
    from_pin: bool,
}

/// The deepest reason a branch of the search failed.
///
/// A search that fails everywhere fails many times, and the useful message is
/// almost always the one from furthest in: the shallow failures are "and that
/// did not work either" restated once per candidate.
#[derive(Default)]
struct Failure {
    depth: usize,
    reason: Option<String>,
}

impl Failure {
    fn record(&mut self, depth: usize, reason: String) {
        if self.reason.is_none() || depth >= self.depth {
            self.depth = depth;
            self.reason = Some(reason);
        }
    }
}

/// Everything the search consults but never changes.
struct Context<'a> {
    workspace: &'a Workspace,
    toolchain_version: &'a Version,
    sources: &'a Sources,
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
    let context = Context {
        workspace,
        toolchain_version,
        sources,
    };
    let root_id = PackageId {
        name: root.name.clone(),
        version: root.version.clone(),
        source: SourceId::Path(root.root.clone()),
    };
    let needs = context.needs_of(root, &root_id)?;

    let mut chosen = BTreeMap::new();
    chosen.insert(
        root_id.name.clone(),
        Chosen {
            id: root_id.clone(),
            checksum: None,
            project: Some(root.clone()),
            entry: None,
            needs: needs.clone(),
        },
    );

    let mut failure = Failure::default();
    let solved = context
        .search(VecDeque::from(needs.clone()), chosen, 0, &mut failure)?
        .ok_or_else(|| {
            failure
                .reason
                .unwrap_or_else(|| format!("cannot resolve the dependencies of `{}`", root_id.name))
        })?;
    let solved = context.settle(solved)?;

    // Language items come from the root's own direct dependencies, in the order
    // its manifest lists them, so two of them are reported the same way twice.
    let mut language_items = Vec::new();
    let mut language_item_source = None;
    for need in &needs {
        let chosen = &solved[&need.name];
        collect_language_items(
            &need.name,
            &chosen.id,
            chosen.project.as_ref(),
            &mut language_items,
            &mut language_item_source,
        )?;
    }

    let packages: BTreeMap<String, ResolvedPackage> = solved
        .into_iter()
        .map(|(name, chosen)| {
            (
                name,
                ResolvedPackage {
                    dependencies: chosen.dependencies(),
                    id: chosen.id,
                    project: chosen.project,
                    checksum: chosen.checksum,
                },
            )
        })
        .collect();
    reject_cycles(&packages, &root_id.name)?;

    Ok(Resolution {
        root: root_id,
        packages,
        language_items,
    })
}

impl Context<'_> {
    /// Satisfy `pending` against `chosen`, or answer that this branch cannot.
    ///
    /// `Err` is a failure of the machinery — an unreadable manifest, a registry
    /// nobody configured — and stops everything. `Ok(None)` is this branch not
    /// working out, which is what backtracking is for.
    fn search(
        &self,
        mut pending: VecDeque<Need>,
        chosen: BTreeMap<String, Chosen>,
        depth: usize,
        failure: &mut Failure,
    ) -> Result<Option<BTreeMap<String, Chosen>>, String> {
        let Some(need) = pending.pop_front() else {
            return Ok(Some(chosen));
        };

        if let Some(existing) = chosen.get(&need.name) {
            if need.requirement.matches(&existing.id.version)
                && need.want.matches(&existing.id.source)
            {
                return self.search(pending, chosen, depth + 1, failure);
            }
            let options = self.options(&need, false)?;
            failure.record(depth, self.disagreement(&need, existing, &options)?);
            return Ok(None);
        }

        let options = self.options(&need, true)?;
        if let Some(solved) = self.try_each(&need, &options, &pending, &chosen, depth, failure)? {
            return Ok(Some(solved));
        }
        // The lockfile's answer did not work out. It is still the answer this
        // graph is supposed to have, so it was tried first; the rest of what
        // the registry publishes is tried only now.
        if options.from_pin {
            let all = self.options(&need, false)?;
            let already = options.all.first().map(|chosen| chosen.id.clone());
            let rest = Options {
                all: all
                    .all
                    .into_iter()
                    .filter(|candidate| Some(&candidate.id) != already.as_ref())
                    .collect(),
                ..all
            };
            if let Some(solved) = self.try_each(&need, &rest, &pending, &chosen, depth, failure)? {
                return Ok(Some(solved));
            }
        }
        Ok(None)
    }

    fn try_each(
        &self,
        need: &Need,
        options: &Options,
        pending: &VecDeque<Need>,
        chosen: &BTreeMap<String, Chosen>,
        depth: usize,
        failure: &mut Failure,
    ) -> Result<Option<BTreeMap<String, Chosen>>, String> {
        let matching = self.matching(need, options);
        if matching.is_empty() && !options.from_pin {
            failure.record(depth, self.no_candidate(need, options));
            return Ok(None);
        }
        for candidate in matching {
            let mut pending = pending.clone();
            pending.extend(candidate.needs.iter().cloned());
            let mut chosen = chosen.clone();
            chosen.insert(need.name.clone(), candidate.clone());
            if let Some(solved) = self.search(pending, chosen, depth + 1, failure)? {
                return Ok(Some(solved));
            }
        }
        Ok(None)
    }

    /// The candidates that satisfy a requirement, newest first.
    fn matching<'a>(&self, need: &Need, options: &'a Options) -> Vec<&'a Chosen> {
        options
            .all
            .iter()
            .filter(|candidate| need.requirement.matches(&candidate.id.version))
            .filter(|candidate| match self.sources.precise(&need.name) {
                Some(exact) => candidate.id.version == *exact,
                None => true,
            })
            .collect()
    }

    /// Everything a source offers for a name, newest first.
    ///
    /// With `use_pin`, a lockfile entry that still applies is the whole answer
    /// and nothing is fetched to produce it — which is what lets a resolved
    /// project resolve again with no network and no index.
    fn options(&self, need: &Need, use_pin: bool) -> Result<Options, String> {
        let single = |chosen: Chosen| Options {
            all: vec![chosen],
            yanked: Vec::new(),
            from_pin: false,
        };
        match &need.want {
            Want::Path(root) => Ok(single(self.path_package(need, root)?)),
            Want::Toolchain => Ok(single(self.toolchain_package(need)?)),
            Want::Git { url, reference } => Ok(single(self.git_package(need, url, reference)?)),
            Want::Registry { index } => {
                if use_pin {
                    if let Some(pin) = self.applicable_pin(need, index) {
                        return Ok(Options {
                            all: vec![self.registry_package_from_lock(need, pin)?],
                            yanked: Vec::new(),
                            from_pin: true,
                        });
                    }
                }
                let mut all = Vec::new();
                let mut yanked = Vec::new();
                for entry in self.sources.published(&need.name, index)? {
                    if entry.yanked {
                        yanked.push(entry.version.clone());
                        continue;
                    }
                    all.push(self.registry_package(need, index, entry)?);
                }
                Ok(Options {
                    all,
                    yanked,
                    from_pin: false,
                })
            }
        }
    }

    /// The lockfile's entry for this name, if it is still an answer to it.
    fn applicable_pin(&self, need: &Need, index: &str) -> Option<&Pin> {
        let pin = self.sources.pinned(&need.name)?;
        let applies = need.want.matches(&pin.source)
            && need.requirement.matches(&pin.version)
            && self
                .sources
                .precise(&need.name)
                .is_none_or(|exact| *exact == pin.version);
        let _ = index;
        applies.then_some(pin)
    }

    fn path_package(&self, need: &Need, root: &Path) -> Result<Chosen, String> {
        let project = self.load_path_dependency(root)?;
        // `D-035`: the key in `[dependencies]` *is* the package name, because
        // the name is what the namespace and the lock are built from. An alias
        // that differed from it would give one package two names.
        if project.name != need.name {
            return Err(format!(
                "{}: `{}` declares dependency `{}`, but the package at `{}` is named `{}`; the key in `[dependencies]` must be the package name",
                codes::WRONG_NAME,
                need.dependent,
                need.name,
                project.root.display(),
                project.name
            ));
        }
        let id = PackageId {
            name: project.name.clone(),
            version: project.version.clone(),
            source: SourceId::Path(project.root.clone()),
        };
        let needs = self.needs_of(&project, &id)?;
        Ok(Chosen {
            id,
            checksum: None,
            project: Some(project),
            entry: None,
            needs,
        })
    }

    fn toolchain_package(&self, need: &Need) -> Result<Chosen, String> {
        let Some(bundled) = toolchain_package(&need.name) else {
            let names: Vec<&str> = crate::std_library::TOOLCHAIN_PACKAGES
                .iter()
                .map(|package| package.name)
                .collect();
            return Err(format!(
                "{}: dependency `{}` cannot use the toolchain source; the bundled packages are {}",
                codes::TOOLCHAIN_SOURCE,
                need.name,
                names.join(" and ")
            ));
        };
        let (_, checksum) = std_archive(bundled, self.toolchain_version)?;
        let id = PackageId {
            name: bundled.name.to_owned(),
            version: self.toolchain_version.clone(),
            source: SourceId::Toolchain,
        };
        let project = replacement(self.workspace, &id, &checksum)?;
        // A bundled package may depend on another bundled package (`D-082`).
        // A replacement declares its own needs; the bundled copy's are the
        // table's.
        let needs = match &project {
            Some(project) => self.needs_of(project, &id)?,
            None => bundled
                .dependencies
                .iter()
                .map(|name| Need {
                    dependent: bundled.name.to_owned(),
                    name: (*name).to_owned(),
                    // The same as what the generated manifest writes —
                    // `core = { toolchain = true }`, no version. `Want` pins
                    // the source, and the toolchain has exactly one of these.
                    requirement: VersionReq::any(),
                    want: Want::Toolchain,
                })
                .collect(),
        };
        Ok(Chosen {
            id,
            checksum: Some(checksum),
            project,
            entry: None,
            needs,
        })
    }

    fn git_package(
        &self,
        need: &Need,
        url: &str,
        reference: &GitReference,
    ) -> Result<Chosen, String> {
        let pin = self.sources.pin_git(&need.name, url, reference)?;
        let id = PackageId {
            name: need.name.clone(),
            version: pin.version.clone(),
            source: pin.source.clone(),
        };
        let project = self.materialize(&id, &pin.checksum)?;
        if project.name != id.name || project.version != id.version {
            return Err(format!(
                "{}: `{url}` at {} is `{} v{}`, but it was resolved as `{id}`",
                codes::WRONG_NAME,
                pin.source,
                project.name,
                project.version
            ));
        }
        let needs = self.needs_of(&project, &id)?;
        Ok(Chosen {
            id,
            checksum: Some(pin.checksum),
            project: Some(project),
            entry: None,
            needs,
        })
    }

    /// A registry package the lockfile already pinned.
    ///
    /// Its requirements come from its own manifest rather than from the index,
    /// because the index is what a *new* resolution reads and this one is not
    /// new — the archive is in the store, or vendored, and either way it is
    /// here.
    fn registry_package_from_lock(&self, need: &Need, pin: &Pin) -> Result<Chosen, String> {
        let checksum = pin.checksum.ok_or_else(|| {
            format!(
                "{}: `{}` records `{}` as a registry package with no checksum; delete it and resolve again",
                codes::NO_CHECKSUM,
                crate::lock::LOCK_FILE,
                need.name
            )
        })?;
        let id = PackageId {
            name: need.name.clone(),
            version: pin.version.clone(),
            source: pin.source.clone(),
        };
        let project = self.materialize(&id, &checksum)?;
        let needs = self.needs_of(&project, &id)?;
        Ok(Chosen {
            id,
            checksum: Some(checksum),
            project: Some(project),
            entry: None,
            needs,
        })
    }

    /// A registry package the index offered, not yet downloaded.
    fn registry_package(
        &self,
        need: &Need,
        index: &str,
        entry: IndexEntry,
    ) -> Result<Chosen, String> {
        let id = PackageId {
            name: need.name.clone(),
            version: entry.version.clone(),
            source: SourceId::Registry {
                index: index.to_owned(),
            },
        };
        let needs = entry
            .dependencies
            .iter()
            .map(|dependency| Need {
                dependent: id.name.clone(),
                name: dependency.name.clone(),
                requirement: dependency.requirement.clone(),
                want: match dependency.source {
                    IndexSource::SameIndex => Want::Registry {
                        index: index.to_owned(),
                    },
                    IndexSource::Toolchain => Want::Toolchain,
                },
            })
            .collect();
        Ok(Chosen {
            id,
            checksum: Some(entry.checksum),
            project: None,
            entry: Some(entry),
            needs,
        })
    }

    /// The tree of a fetched package: a vendored copy if one stands in for it,
    /// and the store otherwise. Both are checked against the checksum first.
    fn materialize(&self, id: &PackageId, checksum: &Digest) -> Result<Project, String> {
        match replacement(self.workspace, id, checksum)? {
            Some(project) => Ok(project),
            None => {
                let root = self.sources.checkout(id, checksum)?;
                load_project(Some(root.join(MANIFEST_FILE)))
            }
        }
    }

    /// Download what the search selected from an index, and check that what
    /// arrives is what the index said it would be (`D-055`).
    ///
    /// Only what an index offered is downloaded here. The bundled library also
    /// reaches this point without a project, and deliberately: it lives inside
    /// the compiler, so unpacking it into the store would be fetching something
    /// the toolchain is already holding.
    fn settle(&self, chosen: BTreeMap<String, Chosen>) -> Result<BTreeMap<String, Chosen>, String> {
        let mut settled = BTreeMap::new();
        for (name, mut package) in chosen {
            if package.project.is_none() && package.entry.is_some() {
                let checksum = package
                    .checksum
                    .ok_or_else(|| format!("`{name}` was resolved without a checksum"))?;
                let project = self.materialize(&package.id, &checksum)?;
                if project.name != package.id.name || project.version != package.id.version {
                    return Err(format!(
                        "{}: the index offered `{}`, but the archive it points at is `{} v{}`",
                        codes::INDEX_DISAGREEMENT,
                        package.id,
                        project.name,
                        project.version
                    ));
                }
                let declared = self.needs_of(&project, &package.id)?;
                check_against_index(&package, &declared)?;
                package.needs = declared;
                package.project = Some(project);
            }
            settled.insert(name, package);
        }
        Ok(settled)
    }

    /// Every dependency a manifest declares, as requirements to satisfy.
    fn needs_of(&self, project: &Project, id: &PackageId) -> Result<Vec<Need>, String> {
        let mut needs = Vec::new();
        for (declared, spec) in &project.dependencies {
            validate_package_name(declared)?;
            needs.push(Need {
                dependent: project.name.clone(),
                name: declared.clone(),
                requirement: spec.requirement(),
                want: self.want(declared, spec.source(declared)?, project, id)?,
            });
        }
        Ok(needs)
    }

    /// What a written source means to the package that wrote it.
    fn want(
        &self,
        declared: &str,
        spec: SourceSpec,
        project: &Project,
        id: &PackageId,
    ) -> Result<Want, String> {
        let dependent = &project.name;
        match spec {
            SourceSpec::Toolchain => Ok(Want::Toolchain),
            SourceSpec::Path(relative) => match &id.source {
                // `D-051`: a git package is unpacked into the store, so a
                // relative path from one either escapes the package or names a
                // directory whose absolute path a lock must not record. Both
                // have answers; neither is in this release.
                SourceId::Git { .. } => Err(format!(
                    "{}: `{dependent}` comes from git and declares the `path` dependency `{declared}`; a package fetched from a repository cannot have one yet, because there is no way to write where it lives into a lockfile that another machine could read",
                    codes::GIT_PATH_DEPENDENCY
                )),
                SourceId::Registry { .. } => Err(format!(
                    "{}: `{dependent} v{}` came from a registry and declares the `path` dependency `{declared}`; a published package depends only on its own registry and the toolchain (`D-054`)",
                    codes::UNPUBLISHABLE,
                    id.version
                )),
                _ => Ok(Want::Path(project.root.join(relative))),
            },
            SourceSpec::Git { url, reference } => match &id.source {
                SourceId::Registry { .. } => Err(format!(
                    "{}: `{dependent} v{}` came from a registry and declares the `git` dependency `{declared}`; a published package depends only on its own registry and the toolchain (`D-054`)",
                    codes::UNPUBLISHABLE,
                    id.version
                )),
                _ => Ok(Want::Git { url, reference }),
            },
            SourceSpec::Registry { registry } => match &id.source {
                // A fetched manifest's registry names are its author's local
                // nicknames and mean nothing here, so the only one a published
                // package may write is the one that means "mine" (`D-054`).
                SourceId::Registry { index } if registry == DEFAULT_REGISTRY => {
                    Ok(Want::Registry {
                        index: index.clone(),
                    })
                }
                SourceId::Registry { .. } => Err(format!(
                    "{}: `{dependent} v{}` came from a registry and takes `{declared}` from the registry it calls `{registry}`; a name written in a published manifest means nothing on the machine that fetched it (`D-054`)",
                    codes::UNPUBLISHABLE,
                    id.version
                )),
                _ => Ok(Want::Registry {
                    index: self.sources.registries().named(&registry)?.index().to_owned(),
                }),
            },
        }
    }

    /// Load a `path` dependency.
    ///
    /// A path that lands on a workspace member resolves to that member rather
    /// than being read again as a stranger: a member's manifest may inherit
    /// fields from the workspace, and re-reading it from here would either miss
    /// them or have to rediscover the workspace to find them.
    fn load_path_dependency(&self, root: &Path) -> Result<Project, String> {
        if let Ok(canonical) = root.canonicalize() {
            if let Some(member) = self.workspace.member_at(&canonical) {
                return Ok(member.clone());
            }
        }
        let manifest = if root.is_dir() {
            root.join(MANIFEST_FILE)
        } else {
            root.to_path_buf()
        };
        load_project(Some(manifest))
    }

    /// Why a name already chosen is not what this requirement asked for.
    fn disagreement(
        &self,
        need: &Need,
        existing: &Chosen,
        options: &Options,
    ) -> Result<String, String> {
        let Some(best) = self.matching(need, options).first().copied() else {
            return Ok(self.no_candidate(need, options));
        };
        if best.id.source != existing.id.source {
            return Ok(format!(
                "{}: `{}` is required from two sources: `{}` and {}. A package name resolves from one source in a graph (`D-038`)",
                codes::TWO_SOURCES,
                need.name, existing.id.source, need.want
            ));
        }
        Ok(format!(
            "{}: `{}` is required at two versions: {} and {}. Two incompatible versions of one package cannot coexist in a graph",
            codes::TWO_VERSIONS,
            need.name, existing.id.version, best.id.version
        ))
    }

    /// Why nothing a source offers satisfies a requirement.
    fn no_candidate(&self, need: &Need, options: &Options) -> String {
        let asked = format!(
            "{}: cannot select a version of `{}`: `{}` requires {}",
            codes::NO_VERSION,
            need.name,
            need.dependent,
            need.requirement
        );
        // A yanked version that would otherwise have been the answer is the
        // useful thing to say — "the only candidate is 1.0.0" hides it.
        let withdrawn: Vec<Version> = options
            .yanked
            .iter()
            .filter(|version| need.requirement.matches(version))
            .cloned()
            .collect();
        if !withdrawn.is_empty() {
            return format!(
                "{}: {asked}, and every version that would satisfy it is yanked ({})",
                codes::ALL_YANKED,
                versions(&withdrawn)
            );
        }
        match options.all.as_slice() {
            [] => format!("{asked}, but {} offers no `{}`", need.want, need.name),
            [only] => format!("{asked}, but the only candidate is {}", only.id.version),
            many => format!(
                "{asked}, but {} publishes {}",
                need.want,
                versions(
                    &many
                        .iter()
                        .map(|candidate| candidate.id.version.clone())
                        .collect::<Vec<_>>()
                )
            ),
        }
    }
}

fn versions(versions: &[Version]) -> String {
    versions
        .iter()
        .map(|version| version.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// `D-055`: the index makes resolution fast and is trusted for nothing else,
/// so what the archive turns out to declare has to be what selected it.
fn check_against_index(package: &Chosen, declared: &[Need]) -> Result<(), String> {
    let Some(entry) = &package.entry else {
        return Ok(());
    };
    let mut published: Vec<(String, String)> = entry
        .dependencies
        .iter()
        .map(|dependency| (dependency.name.clone(), dependency.requirement.to_string()))
        .collect();
    let mut actual: Vec<(String, String)> = declared
        .iter()
        .map(|need| (need.name.clone(), need.requirement.to_string()))
        .collect();
    published.sort();
    actual.sort();
    if published == actual {
        return Ok(());
    }
    let render = |entries: &[(String, String)]| {
        if entries.is_empty() {
            "nothing".to_owned()
        } else {
            entries
                .iter()
                .map(|(name, requirement)| format!("`{name}` {requirement}"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    Err(format!(
        "{}: the index says `{}` requires {}, but its manifest requires {}. The index is what selected it, so the two have to agree",
        codes::INDEX_DISAGREEMENT,
        package.id,
        render(&published),
        render(&actual)
    ))
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
            "{}: `{source}` is replaced by the vendored packages in `{}`, but `{}` is not there; run `slopium vendor`",
            codes::VENDOR_MISSING,
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
            "{}: the vendored copy at `{}` is `{} v{}`, but it stands in for `{id}`",
            codes::VENDOR_MISSING,
            root.display(),
            project.name,
            project.version
        ));
    }
    Ok(Some(project))
}

/// Language items come from whichever direct dependency declares them.
///
/// This used to key off the alias `std`, so a replacement library had to be
/// *called* `std` to be believed. `D-011` says the standard library is an
/// ordinary dependency; what makes it the standard library is that it declares
/// the language items, not what it is named.
fn collect_language_items(
    declared: &str,
    id: &PackageId,
    dependency: Option<&Project>,
    items: &mut Vec<(String, String)>,
    source: &mut Option<String>,
) -> Result<(), String> {
    let contributed = match dependency {
        // A bundled package with no vendored copy standing in for it: what it
        // declares is in the table, not on disk. `core` and `std` declare
        // different paths for the same items (`D-082`), so it is the package's
        // name that decides and not the fact that it is bundled.
        None => language_items_of(&id.name),
        Some(project) if !project.manifest.language_items.is_empty() => project
            .manifest
            .language_items
            .entries()
            .into_iter()
            .map(|(name, path)| (name, format!("{}:{path}", project.name)))
            .collect(),
        Some(_) => return Ok(()),
    };
    if contributed.is_empty() {
        return Ok(());
    }
    if let Some(previous) = source {
        return Err(format!(
            "{}: `{previous}` and `{declared}` both define `[language-items]`; a package graph has one standard library",
            codes::TWO_STDLIBS
        ));
    }
    *source = Some(declared.to_owned());
    *items = contributed;
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
                return Err(format!(
                    "{}: package dependency cycle: {}",
                    codes::CYCLE,
                    cycle.join(" -> ")
                ));
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
        /// What `[registry.default] trusted-keys` says here. Empty is the
        /// v0.4.4 behaviour and is what every test that predates signing gets.
        trusted: std::cell::RefCell<Vec<String>>,
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
            // The registry directory exists from the start even when nothing is
            // published into it. A registry that publishes no version of a name
            // and a registry whose directory is not there are different
            // failures with different messages, and a fixture that conflated
            // them would test the wrong one.
            fs::create_dir_all(root.join("registry")).unwrap();
            Self {
                root,
                trusted: std::cell::RefCell::new(Vec::new()),
            }
        }

        /// Configure a key this checkout accepts packages from.
        fn trust(&self, key: &crate::signature::PublicKey) {
            self.trusted.borrow_mut().push(key.to_string());
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
        /// developer's own — and never has to be told to clean it up. The
        /// registry beside it is a directory, which is all a registry is.
        fn sources(&self) -> Sources {
            let mut config = crate::manifest::LocalConfig::default();
            config.registry.insert(
                "default".to_owned(),
                crate::manifest::RegistryConfig {
                    index: Some(self.root.join("registry").display().to_string()),
                    trusted_keys: self.trusted.borrow().clone(),
                },
            );
            Sources::new(
                crate::store::Store::at(self.root.join(".store")),
                crate::store::Access::Online,
                false,
            )
            .with_registries(crate::registry::Registries::from_config(&config, &self.root).unwrap())
        }

        /// Publish a package into that registry: an archive under `packages/`
        /// and a line under `index/`, exactly as `docs/packaging.md` says.
        fn publish(&self, name: &str, version: &str, dependencies: &[(&str, &str)]) -> Digest {
            self.publish_signed(name, version, dependencies, None)
        }

        /// The same, signed by a key — or, given `None`, published unsigned,
        /// which is what a registry nobody has configured keys for looks like.
        fn publish_signed(
            &self,
            name: &str,
            version: &str,
            dependencies: &[(&str, &str)],
            key: Option<&crate::signature::PrivateKey>,
        ) -> Digest {
            let version = Version::parse(version).unwrap();
            let table = dependencies
                .iter()
                .map(|(name, requirement)| format!("{name} = \"{requirement}\"\n"))
                .collect::<String>();
            let root = self
                .root
                .join("published")
                .join(format!("{name}-{version}"));
            fs::create_dir_all(root.join("src")).unwrap();
            fs::write(
                root.join(MANIFEST_FILE),
                format!(
                    "[package]\nname = \"{name}\"\nversion = \"{version}\"\nsource = \"src\"\nentry = \"src/lib.slp\"\n\n[dependencies]\n{table}"
                ),
            )
            .unwrap();
            fs::write(root.join("src/lib.slp"), "(fn unused () -> i32 0)\n").unwrap();

            let (bytes, checksum) =
                crate::archive::directory_archive(&root, &prefix_for(name, &version)).unwrap();
            let signature = key.map(|key| key.sign(name, &version, &checksum));
            self.serve(
                name,
                crate::registry::IndexEntry {
                    name: name.to_owned(),
                    version,
                    dependencies: dependencies
                        .iter()
                        .map(|(name, requirement)| crate::registry::IndexDependency {
                            name: (*name).to_owned(),
                            requirement: VersionReq::parse(requirement).unwrap(),
                            source: IndexSource::SameIndex,
                        })
                        .collect(),
                    checksum,
                    yanked: false,
                    signature,
                },
                &bytes,
            );
            checksum
        }

        /// Put one entry and its archive where a registry keeps them.
        fn serve(&self, name: &str, entry: crate::registry::IndexEntry, archive: &[u8]) {
            let registry = self.root.join("registry");
            let index = registry
                .join(crate::registry::INDEX_DIRECTORY)
                .join(crate::registry::index_path(name).unwrap());
            fs::create_dir_all(index.parent().unwrap()).unwrap();
            let mut lines = fs::read_to_string(&index).unwrap_or_default();
            lines.push_str(&entry.render().unwrap());
            lines.push('\n');
            fs::write(&index, lines).unwrap();

            let archive_path = registry.join(crate::registry::archive_path(name, &entry.version));
            fs::create_dir_all(archive_path.parent().unwrap()).unwrap();
            fs::write(archive_path, archive).unwrap();
            if let Some(signature) = entry.signature {
                fs::write(
                    registry.join(crate::registry::signature_path(name, &entry.version)),
                    format!("{signature}\n"),
                )
                .unwrap();
            }
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
        let std = toolchain_package("std").expect("the std package");
        let (_, digest) = std_archive(std, &Version::new(0, 3, 7)).unwrap();
        assert_eq!(resolution.packages["std"].checksum, Some(digest));
        assert_eq!(resolution.packages["application"].checksum, None);
    }

    /// The bundled library ships inside the compiler, so resolving it must not
    /// put it in the store — a store nobody can write to is exactly the
    /// situation a language server on a locked-down machine is in.
    #[test]
    fn the_bundled_library_is_not_fetched_into_the_store() {
        let workspace = Workspace::new("toolchain-unfetched");
        workspace.package("application", "1.0.0", "std = { toolchain = true }\n");
        let manifest = workspace.root.join("application").join(MANIFEST_FILE);
        let loaded = crate::workspace::load_workspace(Some(manifest)).unwrap();
        let project = loaded.select(None, false).unwrap()[0].clone();
        let sources = Sources::new(
            crate::store::Store::at("/nonexistent"),
            crate::store::Access::Offline,
            false,
        );
        let resolution =
            super::resolve(&project, &loaded, &Version::new(0, 3, 7), &sources).unwrap();
        // `std` depends on `core`, so asking for one resolves both (`D-082`).
        assert_eq!(names(&resolution), vec!["core", "std"]);
        assert!(resolution.packages["std"].project.is_none());
        assert!(resolution.packages["core"].project.is_none());
    }

    /// Vendoring may change where bytes are read from and nothing else: the
    /// package keeps its source, its identity and its lock entry (`D-047`).
    #[test]
    fn a_replaced_source_is_read_from_the_vendor_directory() {
        let workspace = Workspace::new("replacement");
        workspace.package("application", "1.0.0", "std = { toolchain = true }\n");
        let version = Version::new(0, 3, 7);
        let std = toolchain_package("std").expect("the std package");
        // Replacing the toolchain source replaces all of it, so `core` has to
        // be vendored beside `std` (`D-082`).
        for bundled in crate::std_library::TOOLCHAIN_PACKAGES {
            let entries = crate::std_library::std_entries(bundled, &version);
            let root = workspace
                .root
                .join(format!("application/vendor/{}", bundled.name));
            crate::store::unpack(&entries, &root).unwrap();
        }
        workspace.write_manifest(
            "application/.slopium/config.toml",
            "[source.toolchain]\nreplace-with = \"vendored\"\n\n[source.vendored]\ndirectory = \"vendor\"\n",
        );

        let resolution = workspace.resolve("application").unwrap();
        let standard = &resolution.packages["std"];
        assert_eq!(standard.id.source, SourceId::Toolchain);
        assert_eq!(
            standard.checksum,
            Some(std_archive(std, &version).unwrap().1)
        );
        assert_eq!(
            standard
                .project
                .as_ref()
                .map(|project| project.root.clone()),
            Some(workspace.root.join("application/vendor/std"))
        );
        assert_eq!(resolution.language_items, language_items_of("std"));

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
        assert!(error.contains("SL1076"), "{error}");
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

    /// `D-038`: one name comes from one place. Two directories that both call
    /// themselves `shared` are two sources, and saying so names the thing to
    /// fix — which directory the graph was not expecting.
    #[test]
    fn one_name_from_two_directories_is_rejected() {
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
        assert!(error.contains("SL1031"), "{error}");
        assert!(error.contains("SL1031"), "{error}");
        assert!(error.contains("two sources"), "{error}");
        assert!(error.contains("old-shared"), "{error}");
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
        assert!(error.contains("SL1070"), "{error}");
        assert!(error.contains("package dependency cycle"), "{error}");
    }

    #[test]
    fn a_key_that_is_not_the_package_name_is_rejected() {
        let workspace = Workspace::new("alias");
        workspace.package("mathlib", "1.0.0", "");
        workspace.package("application", "1.0.0", "math = { path = \"../mathlib\" }\n");

        let error = workspace.resolve("application").unwrap_err();
        assert!(error.contains("SL1071"), "{error}");
        assert!(error.contains("must be the package name"), "{error}");
    }

    #[test]
    fn the_toolchain_supplies_language_items() {
        let workspace = Workspace::new("toolchain");
        workspace.package("application", "1.0.0", "std = { toolchain = true }\n");

        let resolution = workspace.resolve("application").unwrap();
        assert_eq!(names(&resolution), vec!["core", "std"]);
        // The root depends on `std` directly and on `core` only through it, so
        // it is `std` that declares the items — through its own `prelude`,
        // which re-exports `core`'s (`D-082`).
        assert!(resolution
            .language_items
            .contains(&("option".to_owned(), "std:prelude:Option".to_owned())));
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
        assert!(error.contains("SL1074"), "{error}");
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

    /// Maximal selection, finally with something to select from: two versions
    /// are published and the newer compatible one is what the graph gets.
    #[test]
    fn a_registry_dependency_takes_the_newest_compatible_version() {
        let workspace = Workspace::new("registry");
        workspace.publish("geometry", "1.0.0", &[]);
        let checksum = workspace.publish("geometry", "1.2.0", &[]);
        workspace.publish("geometry", "2.0.0", &[]);
        workspace.package("application", "1.0.0", "geometry = \"^1\"\n");

        let resolution = workspace.resolve("application").unwrap();
        let geometry = &resolution.packages["geometry"];
        assert_eq!(geometry.id.version, Version::new(1, 2, 0));
        assert_eq!(geometry.checksum, Some(checksum));
        assert_eq!(
            geometry.id.source,
            SourceId::Registry {
                index: workspace.root.join("registry").display().to_string(),
            }
        );
        // The archive was downloaded and unpacked, so the compiler is handed a
        // directory like it is for every other source.
        assert!(geometry
            .project
            .as_ref()
            .is_some_and(|project| project.root.join(MANIFEST_FILE).is_file()));
    }

    /// The patch that finally puts weight on backtracking. Greedy selection
    /// takes `left 1.1.0`, which needs `shared ^2`, and then cannot satisfy
    /// `right`; the answer is the older `left`, which no amount of ordering
    /// finds without going back.
    #[test]
    fn a_diamond_backtracks_to_an_older_dependent() {
        let workspace = Workspace::new("backtrack");
        workspace.publish("shared", "1.0.0", &[]);
        workspace.publish("shared", "2.0.0", &[]);
        workspace.publish("left", "1.0.0", &[("shared", "^1")]);
        workspace.publish("left", "1.1.0", &[("shared", "^2")]);
        workspace.publish("right", "1.0.0", &[("shared", "^1")]);
        workspace.package("application", "1.0.0", "left = \"^1\"\nright = \"^1\"\n");

        let resolution = workspace.resolve("application").unwrap();
        assert_eq!(names(&resolution), vec!["left", "right", "shared"]);
        assert_eq!(
            resolution.packages["left"].id.version,
            Version::new(1, 0, 0)
        );
        assert_eq!(
            resolution.packages["shared"].id.version,
            Version::new(1, 0, 0)
        );
    }

    /// `D-036`: when backtracking cannot make one name work at one version,
    /// that is the answer, and the message says which two versions were wanted.
    #[test]
    fn two_versions_of_one_name_are_rejected() {
        let workspace = Workspace::new("two-registry-versions");
        workspace.publish("shared", "1.0.0", &[]);
        workspace.publish("shared", "2.0.0", &[]);
        workspace.publish("left", "1.0.0", &[("shared", "^2")]);
        workspace.package("application", "1.0.0", "left = \"^1\"\nshared = \"^1\"\n");

        let error = workspace.resolve("application").unwrap_err();
        assert!(error.contains("SL1072"), "{error}");
        assert!(error.contains("two versions"), "{error}");
        assert!(error.contains("shared"), "{error}");
    }

    /// `D-055`: yanking is a statement about new resolutions, so a yanked
    /// version is skipped — and a requirement that has nothing else says so.
    #[test]
    fn a_yanked_version_is_not_selected() {
        let workspace = Workspace::new("yanked");
        workspace.publish("geometry", "1.0.0", &[]);
        let (bytes, checksum) = published_bytes(&workspace, "geometry", "1.1.0");
        workspace.serve(
            "geometry",
            crate::registry::IndexEntry {
                name: "geometry".to_owned(),
                version: Version::new(1, 1, 0),
                dependencies: Vec::new(),
                checksum,
                yanked: true,
                signature: None,
            },
            &bytes,
        );
        workspace.package("application", "1.0.0", "geometry = \"^1\"\n");
        assert_eq!(
            workspace.resolve("application").unwrap().packages["geometry"]
                .id
                .version,
            Version::new(1, 0, 0)
        );

        workspace.package("application", "1.0.0", "geometry = \"^1.1\"\n");
        let error = workspace.resolve("application").unwrap_err();
        assert!(error.contains("SL1035"), "{error}");
        assert!(error.contains("yanked"), "{error}");
    }

    /// `D-055`: the index makes resolution fast and is trusted for nothing
    /// else, so an entry that misdescribes its own archive is caught.
    #[test]
    fn an_index_that_disagrees_with_its_archive_is_refused() {
        let workspace = Workspace::new("lying-index");
        workspace.publish("units", "1.0.0", &[]);
        // The archive requires `units`; the index says it requires nothing.
        let (bytes, checksum) = published_with(&workspace, "geometry", "1.0.0", "units = \"^1\"\n");
        workspace.serve(
            "geometry",
            crate::registry::IndexEntry {
                name: "geometry".to_owned(),
                version: Version::new(1, 0, 0),
                dependencies: Vec::new(),
                checksum,
                yanked: false,
                signature: None,
            },
            &bytes,
        );
        workspace.package("application", "1.0.0", "geometry = \"^1\"\n");

        let error = workspace.resolve("application").unwrap_err();
        assert!(error.contains("SL1033"), "{error}");
        assert!(error.contains("`units`"), "{error}");
    }

    /// `D-054`: a published package depends on its own registry and the
    /// toolchain, because nothing else survives being read on another machine.
    #[test]
    fn a_published_package_may_not_declare_a_path_dependency() {
        let workspace = Workspace::new("published-path");
        let (bytes, checksum) = published_with(
            &workspace,
            "geometry",
            "1.0.0",
            "helper = { path = \"../helper\" }\n",
        );
        workspace.serve(
            "geometry",
            crate::registry::IndexEntry {
                name: "geometry".to_owned(),
                version: Version::new(1, 0, 0),
                dependencies: Vec::new(),
                checksum,
                yanked: false,
                signature: None,
            },
            &bytes,
        );
        workspace.package("application", "1.0.0", "geometry = \"^1\"\n");

        let error = workspace.resolve("application").unwrap_err();
        assert!(error.contains("SL1032"), "{error}");
        assert!(error.contains("`helper`"), "{error}");
    }

    /// `D-053`: there is no built-in registry, so a name nobody configured is
    /// an error rather than a download from wherever.
    #[test]
    fn an_unconfigured_registry_is_refused() {
        let workspace = Workspace::new("unconfigured");
        workspace.package(
            "application",
            "1.0.0",
            "geometry = { version = \"^1\", registry = \"internal\" }\n",
        );
        let error = workspace.resolve("application").unwrap_err();
        assert!(error.contains("SL1030"), "{error}");
        assert!(error.contains("internal"), "{error}");
    }

    /// A registry publishing nothing under a name is not a mystery, and the
    /// message says which registry was asked.
    #[test]
    fn a_name_the_registry_does_not_publish_is_reported() {
        let workspace = Workspace::new("unpublished");
        workspace.package("application", "1.0.0", "geometry = \"^1\"\n");
        let error = workspace.resolve("application").unwrap_err();
        assert!(
            error.contains("cannot select a version of `geometry`"),
            "{error}"
        );
        assert!(error.contains("offers no"), "{error}");
    }

    /// The other half of the one above: a registry directory that is not there
    /// is not a registry that publishes nothing. Answering "it offers no
    /// `geometry`" would send somebody looking for a package when what is
    /// wrong is a path.
    #[test]
    fn a_registry_directory_that_is_not_there_says_so() {
        let workspace = Workspace::new("no-registry-directory");
        workspace.package("application", "1.0.0", "geometry = \"^1\"\n");
        fs::remove_dir_all(workspace.root.join("registry")).unwrap();
        let error = workspace.resolve("application").unwrap_err();
        assert!(error.contains("SL1030"), "{error}");
        assert!(error.contains("no such directory"), "{error}");
    }

    /// `D-057`: keys configured, key used, package built. The signature is
    /// checked at checkout, so reaching a resolved graph is passing it.
    #[test]
    fn a_package_signed_by_a_trusted_key_resolves() {
        let key = crate::signature::PrivateKey::generate().unwrap();
        let workspace = Workspace::new("signed");
        workspace.trust(&key.public());
        workspace.publish_signed("geometry", "1.0.0", &[], Some(&key));
        workspace.package("application", "1.0.0", "geometry = \"^1\"\n");
        let resolution = workspace.resolve("application").unwrap();
        assert_eq!(
            resolution.packages["geometry"].id.version,
            Version::new(1, 0, 0)
        );
    }

    /// The ordinary case of a publisher rotating a key, and the reason a
    /// signature carries the key that made it (`D-056`): the message can name
    /// the key to add.
    #[test]
    fn a_package_signed_by_an_unlisted_key_is_refused() {
        let publisher = crate::signature::PrivateKey::generate().unwrap();
        let expected = crate::signature::PrivateKey::generate().unwrap();
        let workspace = Workspace::new("unlisted");
        workspace.trust(&expected.public());
        workspace.publish_signed("geometry", "1.0.0", &[], Some(&publisher));
        workspace.package("application", "1.0.0", "geometry = \"^1\"\n");
        let error = workspace.resolve("application").unwrap_err();
        assert!(error.contains("SL1042"), "{error}");
        assert!(
            error.contains(&publisher.public().to_string()),
            "the key to add is named: {error}"
        );
    }

    /// A trusted key's signature over some other package does not become this
    /// one's by being filed next to it.
    #[test]
    fn a_signature_for_another_package_does_not_verify() {
        let key = crate::signature::PrivateKey::generate().unwrap();
        let workspace = Workspace::new("forged");
        workspace.trust(&key.public());
        let (bytes, checksum) = published_bytes(&workspace, "geometry", "1.0.0");
        workspace.serve(
            "geometry",
            crate::registry::IndexEntry {
                name: "geometry".to_owned(),
                version: Version::new(1, 0, 0),
                dependencies: Vec::new(),
                checksum,
                yanked: false,
                signature: Some(key.sign("units", &Version::new(1, 0, 0), &checksum)),
            },
            &bytes,
        );
        workspace.package("application", "1.0.0", "geometry = \"^1\"\n");
        let error = workspace.resolve("application").unwrap_err();
        assert!(error.contains("SL1041"), "{error}");
    }

    /// Configuring keys is what turns signing on. An unsigned package from a
    /// registry that has them is refused rather than quietly accepted.
    #[test]
    fn an_unsigned_package_is_refused_where_keys_are_configured() {
        let key = crate::signature::PrivateKey::generate().unwrap();
        let workspace = Workspace::new("unsigned");
        workspace.trust(&key.public());
        workspace.publish("geometry", "1.0.0", &[]);
        workspace.package("application", "1.0.0", "geometry = \"^1\"\n");
        let error = workspace.resolve("application").unwrap_err();
        assert!(error.contains("SL1040"), "{error}");
        assert!(error.contains("published unsigned"), "{error}");
    }

    /// `D-057`: no keys is a state somebody chose, and it is the one every
    /// registry written before this release is in.
    #[test]
    fn an_unsigned_package_is_taken_where_no_keys_are_configured() {
        let workspace = Workspace::new("untrusting");
        workspace.publish("geometry", "1.0.0", &[]);
        workspace.package("application", "1.0.0", "geometry = \"^1\"\n");
        assert!(workspace.resolve("application").is_ok());
    }

    /// The bytes and digest of a package as a registry would hold it.
    fn published_bytes(workspace: &Workspace, name: &str, version: &str) -> (Vec<u8>, Digest) {
        published_with(workspace, name, version, "")
    }

    fn published_with(
        workspace: &Workspace,
        name: &str,
        version: &str,
        dependencies: &str,
    ) -> (Vec<u8>, Digest) {
        let parsed = Version::parse(version).unwrap();
        let root = workspace
            .root
            .join("published")
            .join(format!("{name}-{version}"));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join(MANIFEST_FILE),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"{version}\"\nsource = \"src\"\nentry = \"src/lib.slp\"\n\n[dependencies]\n{dependencies}"
            ),
        )
        .unwrap();
        fs::write(root.join("src/lib.slp"), "(fn unused () -> i32 0)\n").unwrap();
        crate::archive::directory_archive(&root, &prefix_for(name, &parsed)).unwrap()
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
