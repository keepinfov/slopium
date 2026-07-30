//! Workspaces: several packages that share one lock, one `target/`, and one
//! resolution.
//!
//! Every command loads a `Workspace`, even for a lone package — that one is a
//! workspace of a single member whose root is the package root, so the lock and
//! the build directory land exactly where they did before workspaces existed.
//! Having one shape means `-p`, the lock path and the target directory are
//! written once rather than twice with a branch between them.

use crate::manifest::{
    find_manifest, read_manifest, starting_manifest, Inheritance, Project, RawManifest,
    WorkspaceSection, MANIFEST_FILE,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A workspace and the packages in it.
#[derive(Clone, Debug)]
pub struct Workspace {
    /// Directory of the manifest carrying `[workspace]`.
    pub root: PathBuf,
    pub manifest_path: PathBuf,
    /// Kept verbatim for the build cache: a member's own manifest text does not
    /// change when a dependency it inherits does.
    pub manifest_source: String,
    pub section: WorkspaceSection,
    /// Members by package name.
    pub members: BTreeMap<String, Project>,
    /// The member the command was invoked in, when it was invoked in one.
    pub current: Option<String>,
}

impl Workspace {
    /// A manifest that defines no package of its own.
    pub fn is_virtual(&self) -> bool {
        !self
            .members
            .values()
            .any(|member| member.manifest_path == self.manifest_path)
    }

    /// Where the single `Slopium.lock` lives.
    pub fn lock_path(&self) -> PathBuf {
        self.root.join(crate::lock::LOCK_FILE)
    }

    /// Where the single `target/` lives.
    pub fn target_dir(&self) -> PathBuf {
        self.root.join("target")
    }

    pub fn member(&self, name: &str) -> Result<&Project, String> {
        self.members.get(name).ok_or_else(|| {
            format!(
                "no package named `{name}` in this workspace; it has {}",
                self.member_list()
            )
        })
    }

    /// The member rooted at `root`, if the workspace has one.
    pub fn member_at(&self, root: &Path) -> Option<&Project> {
        self.members.values().find(|member| member.root == root)
    }

    fn member_list(&self) -> String {
        self.members
            .keys()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Which packages a command acts on.
    ///
    /// `--workspace` is every member; `-p` names one; otherwise it is the
    /// member the command was invoked in. A workspace of one needs no selection
    /// at all, which is what keeps single-package projects unchanged.
    pub fn select(&self, package: Option<&str>, all: bool) -> Result<Vec<&Project>, String> {
        if all {
            if let Some(name) = package {
                return Err(format!(
                    "`--workspace` and `--package {name}` disagree about what to act on"
                ));
            }
            return Ok(self.members.values().collect());
        }
        if let Some(name) = package {
            return Ok(vec![self.member(name)?]);
        }
        if let Some(current) = &self.current {
            return Ok(vec![self.member(current)?]);
        }
        if self.members.len() == 1 {
            return Ok(self.members.values().collect());
        }
        Err(format!(
            "this workspace defines several packages ({}); name one with `--package` or act on all of them with `--workspace`",
            self.member_list()
        ))
    }

    /// Exactly one package, for the commands that cannot act on several.
    pub fn select_one(&self, package: Option<&str>, action: &str) -> Result<&Project, String> {
        let selected = self.select(package, false)?;
        match selected.len() {
            1 => Ok(selected[0]),
            _ => Err(format!(
                "`{action}` acts on one package; name it with `--package`"
            )),
        }
    }
}

/// Load the workspace a manifest belongs to, along with every member.
pub fn load_workspace(manifest_path: Option<PathBuf>) -> Result<Workspace, String> {
    let start = read_manifest(&starting_manifest(manifest_path)?)?;

    // A manifest carrying `[workspace]` is a root; anything else may still be a
    // member of a root further up, which is how `slopium build` works from
    // inside a member directory.
    let (root, current) = match &start.manifest.workspace {
        Some(_) => {
            let current = start
                .manifest
                .package
                .as_ref()
                .map(|package| package.name.clone());
            (start, current)
        }
        None => match find_enclosing_workspace(&start.root)? {
            Some(root) => {
                let current = start
                    .manifest
                    .package
                    .as_ref()
                    .map(|package| package.name.clone());
                (root, current)
            }
            None => {
                let name = start
                    .manifest
                    .package
                    .as_ref()
                    .map(|package| package.name.clone());
                (start, name)
            }
        },
    };

    let section = root.manifest.workspace.clone().unwrap_or_default();
    let mut members = BTreeMap::new();
    let mut roots: Vec<PathBuf> = Vec::new();
    if root.manifest.package.is_some() {
        roots.push(root.root.clone());
    }
    for pattern in &section.members {
        for directory in expand_member(&root.root, pattern)? {
            roots.push(directory);
        }
    }
    let excluded = excluded_directories(&root.root, &section);

    for directory in roots {
        if excluded.contains(&directory) {
            continue;
        }
        let raw = if directory == root.root {
            root.clone()
        } else {
            read_manifest(&directory.join(MANIFEST_FILE))?
        };
        let member = raw.into_project(Some(Inheritance {
            section: &section,
            root: &root.root,
        }))?;
        if let Some(previous) = members.insert(member.name.clone(), member) {
            return Err(format!(
                "two members of this workspace are named `{}`: `{}` and one more",
                previous.name,
                previous.root.display()
            ));
        }
    }

    // The package the command was invoked in must be one of them; a package
    // that merely sits inside a workspace directory without being listed is a
    // stranger, and silently building it under the workspace's lock would be a
    // surprise.
    if let Some(name) = &current {
        if !members.contains_key(name) {
            return Err(format!(
                "`{name}` is not a member of the workspace at `{}`; add it to `[workspace] members` or move it out",
                root.root.display()
            ));
        }
    }

    Ok(Workspace {
        root: root.root,
        manifest_path: root.manifest_path,
        manifest_source: root.manifest_source,
        section,
        members,
        current,
    })
}

/// Load one package, through whatever workspace it belongs to.
///
/// This is what a path dependency outside the current workspace goes through,
/// so a dependency that is itself a member of another workspace still gets its
/// inherited fields.
pub fn load_project(manifest_path: Option<PathBuf>) -> Result<Project, String> {
    let path = starting_manifest(manifest_path)?;
    let raw = read_manifest(&path)?;
    let inherited = match &raw.manifest.workspace {
        Some(section) => Some((section.clone(), raw.root.clone())),
        None => find_enclosing_workspace(&raw.root)?.and_then(|enclosing| {
            enclosing
                .manifest
                .workspace
                .clone()
                .map(|section| (section, enclosing.root))
        }),
    };
    let inheritance = inherited
        .as_ref()
        .map(|(section, root)| Inheritance { section, root });
    raw.into_project(inheritance)
}

/// Walk up from a package directory looking for a workspace that lists it.
fn find_enclosing_workspace(package_root: &Path) -> Result<Option<RawManifest>, String> {
    let mut directory = package_root.parent().map(Path::to_path_buf);
    while let Some(candidate) = directory {
        let Some(manifest_path) = find_manifest(&candidate) else {
            return Ok(None);
        };
        let raw = read_manifest(&manifest_path)?;
        if let Some(section) = &raw.manifest.workspace {
            if lists_member(&raw.root, section, package_root)? {
                return Ok(Some(raw));
            }
        }
        directory = raw.root.parent().map(Path::to_path_buf);
    }
    Ok(None)
}

fn lists_member(
    root: &Path,
    section: &WorkspaceSection,
    package_root: &Path,
) -> Result<bool, String> {
    if excluded_directories(root, section).contains(&package_root.to_path_buf()) {
        return Ok(false);
    }
    for pattern in &section.members {
        if expand_member(root, pattern)?
            .iter()
            .any(|directory| directory == package_root)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// `exclude` entries that exist, canonicalized so they compare against member
/// directories. One that does not exist excludes nothing and is not an error —
/// it is how a workspace keeps a directory out before it is created.
fn excluded_directories(root: &Path, section: &WorkspaceSection) -> Vec<PathBuf> {
    section
        .exclude
        .iter()
        .filter_map(|pattern| root.join(pattern).canonicalize().ok())
        .collect()
}

/// Expand one `members` pattern.
///
/// A final `*` component stands for every subdirectory holding a manifest,
/// which is the only wildcard worth having: `members = ["crates/*"]`. Anything
/// more elaborate is refused rather than half-supported.
fn expand_member(root: &Path, pattern: &str) -> Result<Vec<PathBuf>, String> {
    let (prefix, wildcard) = match pattern.strip_suffix("/*") {
        Some(prefix) => (prefix, true),
        None => (pattern, pattern == "*"),
    };
    let prefix = if wildcard && pattern == "*" {
        ""
    } else {
        prefix
    };
    if prefix.contains('*') {
        return Err(format!(
            "workspace member `{pattern}` uses `*` outside the final component; only a trailing `/*` is understood"
        ));
    }
    let directory = if prefix.is_empty() {
        root.to_path_buf()
    } else {
        root.join(prefix)
    };
    if !wildcard {
        let canonical = directory.canonicalize().map_err(|error| {
            format!(
                "workspace member `{pattern}`: cannot read `{}`: {error}",
                directory.display()
            )
        })?;
        return Ok(vec![canonical]);
    }
    let entries = std::fs::read_dir(&directory).map_err(|error| {
        format!(
            "workspace member `{pattern}`: cannot read `{}`: {error}",
            directory.display()
        )
    })?;
    let mut matched = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("cannot read `{}`: {error}", directory.display()))?;
        let path = entry.path();
        if path.join(MANIFEST_FILE).is_file() {
            matched.push(
                path.canonicalize()
                    .map_err(|error| format!("cannot read `{}`: {error}", path.display()))?,
            );
        }
    }
    matched.sort();
    Ok(matched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Tree {
        root: PathBuf,
    }

    impl Tree {
        fn new(label: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "slopium-workspace-{label}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            if root.exists() {
                fs::remove_dir_all(&root).unwrap();
            }
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn write(&self, relative: &str, contents: &str) -> PathBuf {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            path
        }

        fn package(&self, directory: &str, body: &str) -> PathBuf {
            self.write(
                &format!("{directory}/src/lib.slp"),
                "(fn unused () -> i32 0)\n",
            );
            self.write(&format!("{directory}/{MANIFEST_FILE}"), body)
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn workspace_root(tree: &Tree, body: &str) -> PathBuf {
        tree.write(MANIFEST_FILE, body)
    }

    #[test]
    fn a_lone_package_is_a_workspace_of_one() {
        let tree = Tree::new("lone");
        let manifest = tree.package(
            "app",
            "[package]\nname = \"app\"\nversion = \"1.0.0\"\nentry = \"src/lib.slp\"\n",
        );
        let workspace = load_workspace(Some(manifest)).unwrap();
        assert_eq!(workspace.members.len(), 1);
        assert_eq!(workspace.current.as_deref(), Some("app"));
        assert_eq!(
            workspace.root,
            tree.root.join("app").canonicalize().unwrap()
        );
        assert!(!workspace.is_virtual());
    }

    #[test]
    fn a_member_finds_the_root_above_it() {
        let tree = Tree::new("member");
        workspace_root(
            &tree,
            "[workspace]\nmembers = [\"app\", \"helper\"]\n\n[workspace.package]\nversion = \"2.1.0\"\n",
        );
        tree.package(
            "app",
            "[package]\nname = \"app\"\nversion.workspace = true\nentry = \"src/lib.slp\"\n",
        );
        let helper = tree.package(
            "helper",
            "[package]\nname = \"helper\"\nversion = \"0.1.0\"\nentry = \"src/lib.slp\"\n",
        );

        let workspace = load_workspace(Some(helper)).unwrap();
        assert_eq!(workspace.root, tree.root.canonicalize().unwrap());
        assert_eq!(workspace.current.as_deref(), Some("helper"));
        assert!(workspace.is_virtual());
        assert_eq!(
            workspace.member("app").unwrap().version.to_string(),
            "2.1.0"
        );
    }

    #[test]
    fn a_trailing_star_matches_every_package_below() {
        let tree = Tree::new("glob");
        workspace_root(&tree, "[workspace]\nmembers = [\"crates/*\"]\n");
        tree.package(
            "crates/one",
            "[package]\nname = \"one\"\nversion = \"1.0.0\"\nentry = \"src/lib.slp\"\n",
        );
        tree.package(
            "crates/two",
            "[package]\nname = \"two\"\nversion = \"1.0.0\"\nentry = \"src/lib.slp\"\n",
        );
        let workspace = load_workspace(Some(tree.root.join(MANIFEST_FILE))).unwrap();
        assert_eq!(
            workspace.members.keys().collect::<Vec<_>>(),
            vec!["one", "two"]
        );
        assert_eq!(workspace.current, None);
    }

    #[test]
    fn an_excluded_directory_is_not_a_member() {
        let tree = Tree::new("exclude");
        workspace_root(
            &tree,
            "[workspace]\nmembers = [\"crates/*\"]\nexclude = [\"crates/scratch\"]\n",
        );
        tree.package(
            "crates/one",
            "[package]\nname = \"one\"\nversion = \"1.0.0\"\nentry = \"src/lib.slp\"\n",
        );
        let scratch = tree.package(
            "crates/scratch",
            "[package]\nname = \"scratch\"\nversion = \"1.0.0\"\nentry = \"src/lib.slp\"\n",
        );
        let workspace = load_workspace(Some(tree.root.join(MANIFEST_FILE))).unwrap();
        assert_eq!(workspace.members.keys().collect::<Vec<_>>(), vec!["one"]);

        // And the excluded package still builds on its own terms.
        let alone = load_workspace(Some(scratch)).unwrap();
        assert_eq!(alone.members.len(), 1);
        assert_eq!(alone.current.as_deref(), Some("scratch"));
    }

    #[test]
    fn a_workspace_dependency_is_taken_whole() {
        let tree = Tree::new("inherit-dependency");
        workspace_root(
            &tree,
            "[workspace]\nmembers = [\"app\"]\n\n[workspace.dependencies]\nfoundation = { path = \"foundation\", version = \"^1.0\" }\n",
        );
        tree.package(
            "app",
            "[package]\nname = \"app\"\nversion = \"1.0.0\"\nentry = \"src/lib.slp\"\n\n[dependencies]\nfoundation = { workspace = true }\n",
        );
        let workspace = load_workspace(Some(tree.root.join(MANIFEST_FILE))).unwrap();
        let spec = &workspace.member("app").unwrap().dependencies["foundation"];
        // Rebased onto the workspace root: `app` must not read it as
        // `app/foundation`.
        assert_eq!(
            spec.path.as_deref(),
            Some(&*workspace.root.join("foundation"))
        );
        assert_eq!(spec.requirement().to_string(), "^1.0");
    }

    #[test]
    fn an_inherited_field_the_workspace_does_not_set_is_reported() {
        let tree = Tree::new("missing-inherit");
        workspace_root(&tree, "[workspace]\nmembers = [\"app\"]\n");
        tree.package(
            "app",
            "[package]\nname = \"app\"\nversion.workspace = true\nentry = \"src/lib.slp\"\n",
        );
        let error = load_workspace(Some(tree.root.join(MANIFEST_FILE))).unwrap_err();
        assert!(error.contains("[workspace.package]"), "{error}");
    }

    #[test]
    fn a_package_inside_a_workspace_that_does_not_list_it_is_reported() {
        let tree = Tree::new("stranger");
        workspace_root(&tree, "[workspace]\nmembers = [\"app\"]\n");
        tree.package(
            "app",
            "[package]\nname = \"app\"\nversion = \"1.0.0\"\nentry = \"src/lib.slp\"\n",
        );
        let stranger = tree.package(
            "stranger",
            "[package]\nname = \"stranger\"\nversion = \"1.0.0\"\nentry = \"src/lib.slp\"\n",
        );
        // Not listed and not excluded: it loads as its own workspace, because
        // nothing above claims it.
        let workspace = load_workspace(Some(stranger)).unwrap();
        assert_eq!(
            workspace.members.keys().collect::<Vec<_>>(),
            vec!["stranger"]
        );
    }

    #[test]
    fn selection_needs_a_name_when_the_root_is_virtual() {
        let tree = Tree::new("select");
        workspace_root(&tree, "[workspace]\nmembers = [\"one\", \"two\"]\n");
        tree.package(
            "one",
            "[package]\nname = \"one\"\nversion = \"1.0.0\"\nentry = \"src/lib.slp\"\n",
        );
        tree.package(
            "two",
            "[package]\nname = \"two\"\nversion = \"1.0.0\"\nentry = \"src/lib.slp\"\n",
        );
        let workspace = load_workspace(Some(tree.root.join(MANIFEST_FILE))).unwrap();

        let error = workspace.select(None, false).unwrap_err();
        assert!(error.contains("--package"), "{error}");
        assert_eq!(workspace.select(Some("two"), false).unwrap().len(), 1);
        assert_eq!(workspace.select(None, true).unwrap().len(), 2);
        assert!(workspace.select(Some("three"), false).is_err());
    }
}
