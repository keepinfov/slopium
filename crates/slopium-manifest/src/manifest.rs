//! `Slopium.toml`, parsed in one place (`D-034`).
//!
//! This used to be two structs: `Manifest` in the project manager and a smaller
//! `WorkspaceManifest` in the language server, which parsed a subset of the same
//! file and walked the dependency graph a second time. Anything both of them
//! must agree about belongs outside both (`D-025`).

use crate::source::{GitReference, SourceSpec, DEFAULT_REGISTRY};
use crate::version::{Version, VersionReq};
use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};

/// The name of a manifest file, everywhere it is looked for.
pub const MANIFEST_FILE: &str = "Slopium.toml";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Absent in a virtual manifest — one that defines a workspace and no
    /// package of its own.
    pub package: Option<Package>,
    pub workspace: Option<WorkspaceSection>,
    /// Ordered so that resolution, the lockfile, and cache keys do not depend
    /// on hash iteration order.
    #[serde(default)]
    pub dependencies: BTreeMap<String, DependencySpec>,
    #[serde(default, rename = "language-items")]
    pub language_items: LanguageItemSection,
    #[serde(default)]
    pub build: BuildSection,
    #[serde(default)]
    pub profile: BTreeMap<String, Profile>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Package {
    pub name: String,
    pub version: Inheritable<Version>,
    /// The module a build starts from. A library has no such module — it is
    /// entered through whichever of its modules a dependent takes from — so the
    /// key may be omitted, and omitting it is how a package says it is one.
    pub entry: Option<PathBuf>,
    pub source: Option<PathBuf>,
    /// Everything the package archive holds, when the default is wrong.
    /// Present, it is the whole answer; the manifest is always packaged.
    #[serde(default)]
    pub include: Vec<String>,
    /// What the archive leaves out, on top of what it never carries.
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// `[workspace]`: the packages that share one lock, one `target/`, and one
/// resolution.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceSection {
    /// Member directories relative to the workspace root. A final `*`
    /// component stands for every subdirectory holding a manifest.
    #[serde(default)]
    pub members: Vec<String>,
    /// Directories a `members` pattern would otherwise have matched.
    #[serde(default)]
    pub exclude: Vec<String>,
    /// What a member may take with `workspace = true`.
    #[serde(default)]
    pub dependencies: BTreeMap<String, DependencySpec>,
    /// What a member may take with `<field>.workspace = true`.
    #[serde(default)]
    pub package: WorkspacePackageSection,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspacePackageSection {
    pub version: Option<Version>,
}

/// A `[package]` field that may name the workspace instead of a value.
///
/// Written as the value itself — `version = "1.2.0"` — or as
/// `version.workspace = true`, which takes it from `[workspace.package]`.
#[derive(Clone, Debug)]
pub enum Inheritable<T> {
    Set(T),
    FromWorkspace,
}

impl<T> Inheritable<T> {
    /// The value written here, or the workspace's.
    pub fn resolve<'a>(
        &'a self,
        field: &str,
        from_workspace: Option<&'a T>,
    ) -> Result<&'a T, String> {
        match self {
            Self::Set(value) => Ok(value),
            Self::FromWorkspace => from_workspace.ok_or_else(|| {
                format!(
                    "`{field}.workspace = true`, but the workspace sets no `{field}` in `[workspace.package]`"
                )
            }),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Inheritable<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct FieldVisitor<T>(PhantomData<T>);

        impl<'de, T: Deserialize<'de>> Visitor<'de> for FieldVisitor<T> {
            type Value = Inheritable<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a value or `{ workspace = true }`")
            }

            fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
                T::deserialize(de::value::StrDeserializer::new(value)).map(Inheritable::Set)
            }

            fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Self::Value, M::Error> {
                let mut inherited = None;
                while let Some(key) = map.next_key::<String>()? {
                    if key != "workspace" {
                        return Err(de::Error::custom(format!(
                            "unknown key `{key}`; a field taken from the workspace is written `{}`",
                            "<field>.workspace = true"
                        )));
                    }
                    inherited = Some(map.next_value::<bool>()?);
                }
                match inherited {
                    Some(true) => Ok(Inheritable::FromWorkspace),
                    Some(false) => Err(de::Error::custom(
                        "`workspace = false` says nothing; write the value itself",
                    )),
                    None => Err(de::Error::custom("expected `workspace = true`")),
                }
            }
        }

        deserializer.deserialize_any(FieldVisitor(PhantomData))
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanguageItemSection {
    pub option: Option<String>,
    pub result: Option<String>,
    #[serde(rename = "result-ok")]
    pub result_ok: Option<String>,
    #[serde(rename = "result-err")]
    pub result_err: Option<String>,
}

impl LanguageItemSection {
    pub fn is_empty(&self) -> bool {
        self.entries().is_empty()
    }

    pub fn entries(&self) -> Vec<(String, String)> {
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

/// One `[dependencies]` entry.
///
/// A struct rather than the untagged enum this used to be: an untagged enum
/// cannot say which field it disliked, so `{ pth = "../x" }` reported that the
/// table matched no variant instead of naming the typo. The bare string form
/// `dep = "^1.2"` is handled by the `Deserialize` impl below rather than by an
/// untagged enum, for exactly that reason.
#[derive(Clone, Debug, Default)]
pub struct DependencySpec {
    pub path: Option<PathBuf>,
    pub toolchain: Option<bool>,
    /// A repository URL, in any form `git` itself accepts.
    pub git: Option<String>,
    /// Which commit of `git` to take. At most one of these, and none of them
    /// without `git`; absent altogether means the repository's default branch.
    pub branch: Option<String>,
    pub tag: Option<String>,
    pub rev: Option<String>,
    /// Which configured registry to take this from. Absent alongside a
    /// `version` means `default`, which has no built-in URL either (`D-053`).
    pub registry: Option<String>,
    pub version: Option<VersionReq>,
    /// `workspace = true`: take this entry from `[workspace.dependencies]`.
    pub workspace: Option<bool>,
}

/// The table form, kept separate only so the derived reader stays available:
/// the visitor below needs it for `visit_map` while `DependencySpec` itself
/// answers to a string as well.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DependencyTable {
    path: Option<PathBuf>,
    toolchain: Option<bool>,
    git: Option<String>,
    branch: Option<String>,
    tag: Option<String>,
    rev: Option<String>,
    registry: Option<String>,
    version: Option<VersionReq>,
    workspace: Option<bool>,
}

impl<'de> Deserialize<'de> for DependencySpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Entry;

        impl<'de> Visitor<'de> for Entry {
            type Value = DependencySpec;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a version requirement or a dependency table")
            }

            fn visit_str<E: de::Error>(self, text: &str) -> Result<Self::Value, E> {
                Ok(DependencySpec {
                    version: Some(VersionReq::parse(text).map_err(E::custom)?),
                    ..DependencySpec::default()
                })
            }

            fn visit_map<M: MapAccess<'de>>(self, map: M) -> Result<Self::Value, M::Error> {
                let table =
                    DependencyTable::deserialize(de::value::MapAccessDeserializer::new(map))?;
                Ok(DependencySpec {
                    path: table.path,
                    toolchain: table.toolchain,
                    git: table.git,
                    branch: table.branch,
                    tag: table.tag,
                    rev: table.rev,
                    registry: table.registry,
                    version: table.version,
                    workspace: table.workspace,
                })
            }
        }

        deserializer.deserialize_any(Entry)
    }
}

impl DependencySpec {
    /// Every key that names where a dependency comes from, for the checks that
    /// have to talk about "a source" without caring which one.
    fn named_sources(&self) -> Vec<&'static str> {
        [
            ("path", self.path.is_some()),
            ("toolchain", self.toolchain.is_some()),
            ("git", self.git.is_some()),
            ("branch", self.branch.is_some()),
            ("tag", self.tag.is_some()),
            ("rev", self.rev.is_some()),
            ("registry", self.registry.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, given)| given.then_some(name))
        .collect()
    }

    /// Which commit of a repository this entry names.
    fn reference(&self, name: &str) -> Result<GitReference, String> {
        let given: Vec<(&str, &String)> = [
            ("branch", self.branch.as_ref()),
            ("tag", self.tag.as_ref()),
            ("rev", self.rev.as_ref()),
        ]
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key, value)))
        .collect();
        match given.as_slice() {
            [] => Ok(GitReference::DefaultBranch),
            [("branch", branch)] => Ok(GitReference::Branch((*branch).clone())),
            [("tag", tag)] => Ok(GitReference::Tag((*tag).clone())),
            [("rev", rev)] => Ok(GitReference::Rev((*rev).clone())),
            _ => Err(format!(
                "dependency `{name}` names {}; a git dependency takes one commit",
                given
                    .iter()
                    .map(|(key, _)| format!("`{key}`"))
                    .collect::<Vec<_>>()
                    .join(" and ")
            )),
        }
    }
    /// This entry with `workspace = true` replaced by what the workspace says.
    ///
    /// One member and one workspace must not each hold half an entry, so the
    /// inherited form carries nothing of its own. An inherited `path` is
    /// rebased onto the workspace root, because that is what it was written
    /// relative to — every member would otherwise read it as its own neighbour.
    pub fn inherit(
        &self,
        name: &str,
        inheritance: Option<Inheritance<'_>>,
    ) -> Result<Self, String> {
        match self.workspace {
            None => Ok(self.clone()),
            Some(false) => Err(format!(
                "dependency `{name}` has `workspace = false`; drop the key or name a source"
            )),
            Some(true) => {
                if !self.named_sources().is_empty() || self.version.is_some() {
                    return Err(format!(
                        "dependency `{name}` says `workspace = true` and also names a source; the workspace entry is taken whole"
                    ));
                }
                let inheritance = inheritance.ok_or_else(|| {
                    format!("dependency `{name}` says `workspace = true`, but this package is not in a workspace")
                })?;
                let mut spec = inheritance
                    .section
                    .dependencies
                    .get(name)
                    .cloned()
                    .ok_or_else(|| {
                        format!(
                            "dependency `{name}` says `workspace = true`, but `[workspace.dependencies]` has no `{name}`"
                        )
                    })?;
                spec.path = spec.path.map(|path| inheritance.root.join(path));
                Ok(spec)
            }
        }
    }

    /// Which source this entry names, rejecting the combinations that have no
    /// meaning.
    pub fn source(&self, name: &str) -> Result<SourceSpec, String> {
        if self.workspace.is_some() {
            return Err(format!(
                "dependency `{name}` still says `workspace = true` after inheritance"
            ));
        }
        // `branch`, `tag` and `rev` refine `git` rather than naming a source of
        // their own, so they are reported as the mistake they are: a commit of
        // no repository.
        if self.git.is_none() {
            if let Some(dangling) = ["branch", "tag", "rev"]
                .into_iter()
                .find(|key| self.named_sources().contains(key))
            {
                return Err(format!(
                    "dependency `{name}` names `{dangling}` without `git`; there is no repository to take it from"
                ));
            }
        }
        match (&self.path, self.toolchain, &self.git, &self.registry) {
            // A requirement and nothing else is the default registry (`D-053`),
            // which is configuration rather than a URL this toolchain knows.
            (None, None, None, None) => match self.version {
                Some(_) => Ok(SourceSpec::Registry {
                    registry: DEFAULT_REGISTRY.to_owned(),
                }),
                None => Err(format!(
                    "dependency `{name}` names no source; give a version requirement, `path`, `git`, or `toolchain = true`"
                )),
            },
            (None, Some(false), None, None) => Err(format!(
                "dependency `{name}` has `toolchain = false`; name a `path` or a `git` repository instead"
            )),
            (Some(path), None, None, None) => Ok(SourceSpec::Path(path.clone())),
            (None, Some(true), None, None) => Ok(SourceSpec::Toolchain),
            (None, None, Some(url), None) => Ok(SourceSpec::Git {
                url: url.clone(),
                reference: self.reference(name)?,
            }),
            (None, None, None, Some(registry)) => Ok(SourceSpec::Registry {
                registry: registry.clone(),
            }),
            _ => Err(format!(
                "dependency `{name}` names {}; pick one",
                self.named_sources()
                    .iter()
                    .filter(|key| matches!(**key, "path" | "toolchain" | "git" | "registry"))
                    .map(|key| format!("`{key}`"))
                    .collect::<Vec<_>>()
                    .join(" and ")
            )),
        }
    }

    pub fn requirement(&self) -> VersionReq {
        self.version.clone().unwrap_or_else(VersionReq::any)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildSection {
    pub target: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    #[serde(rename = "opt-level")]
    pub opt_level: Option<u8>,
    pub debug: Option<bool>,
    /// Whether to strip the linked binary. Absent means the conventional
    /// default: off for a debug build, on otherwise — a stripped binary and a
    /// debuggable one are opposite intents.
    pub strip: Option<bool>,
    /// `"message"` (default) prints the reason a trap aborted; `"abort"` exits
    /// silently and leaves no error strings in the binary.
    pub panic: Option<String>,
}

/// `.slopium/config.toml`, which is per checkout rather than per package.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalConfig {
    #[serde(default)]
    pub toolchain: Toolchain,
    #[serde(default)]
    pub target: BTreeMap<String, Toolchain>,
    /// Where a source's packages are taken from instead, keyed by the source's
    /// name in the lockfile. Written by `slopium vendor`, and read by nobody
    /// else — replacement is invisible to resolution on purpose (`D-047`).
    #[serde(default)]
    pub source: BTreeMap<String, SourceConfig>,
    /// The registries this checkout has been told about, by the name manifests
    /// use for them. `default` is the one a bare requirement means, and it has
    /// no built-in URL either (`D-053`).
    #[serde(default)]
    pub registry: BTreeMap<String, RegistryConfig>,
}

/// One `[registry.<name>]` table.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegistryConfig {
    /// Where the index tree is: an `https://` URL, a `file://` one, or a path
    /// relative to the workspace root.
    pub index: Option<String>,
    /// Who may sign packages taken from here, as `ed25519:<hex>`. An empty list
    /// means signatures are not checked at all; there is no third state, and in
    /// particular nothing is remembered from a first download (`D-057`).
    #[serde(default, rename = "trusted-keys")]
    pub trusted_keys: Vec<String>,
}

/// One `[source.<name>]` table: either a redirection or a place to redirect to.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceConfig {
    #[serde(rename = "replace-with")]
    pub replace_with: Option<String>,
    /// A directory of vendored packages, one per subdirectory, named by package.
    pub directory: Option<PathBuf>,
}

impl LocalConfig {
    /// The directory a source's packages are vendored in, if it is replaced.
    ///
    /// `root` is what a relative `directory` is written against — the workspace
    /// root, since the configuration belongs to the checkout rather than to any
    /// one package in it.
    pub fn replacement(&self, source: &str, root: &Path) -> Result<Option<PathBuf>, String> {
        let Some(replacement) = self
            .source
            .get(source)
            .and_then(|entry| entry.replace_with.as_deref())
        else {
            return Ok(None);
        };
        let target = self.source.get(replacement).ok_or_else(|| {
            format!("`[source.{source}]` is replaced with `{replacement}`, which is not configured")
        })?;
        let directory = target.directory.as_ref().ok_or_else(|| {
            format!("`[source.{replacement}]` names no `directory` to take packages from")
        })?;
        Ok(Some(root.join(directory)))
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Toolchain {
    pub cc: Option<String>,
}

/// A manifest as written, before any workspace inheritance is applied.
#[derive(Clone, Debug)]
pub struct RawManifest {
    pub root: PathBuf,
    pub manifest_path: PathBuf,
    pub manifest_source: String,
    pub manifest: Manifest,
}

/// What a member may inherit, and where the workspace offering it lives.
#[derive(Clone, Copy)]
pub struct Inheritance<'a> {
    pub section: &'a WorkspaceSection,
    /// The workspace root, which inherited relative paths are written against.
    pub root: &'a Path,
}

impl RawManifest {
    /// The package this manifest defines, with `[workspace]` inheritance
    /// applied — so nothing downstream of here has to know a workspace exists.
    pub fn into_project(self, inheritance: Option<Inheritance<'_>>) -> Result<Project, String> {
        let package = self.manifest.package.clone().ok_or_else(|| {
            format!(
                "`{}` defines a workspace and no package of its own",
                self.manifest_path.display()
            )
        })?;
        validate_package_name(&package.name)?;
        let version = package
            .version
            .resolve(
                "version",
                inheritance.and_then(|inherited| inherited.section.package.version.as_ref()),
            )
            .map_err(|error| format!("`{}`: {error}", self.manifest_path.display()))?
            .clone();
        let mut dependencies = BTreeMap::new();
        for (name, spec) in &self.manifest.dependencies {
            dependencies.insert(
                name.clone(),
                spec.inherit(name, inheritance)
                    .map_err(|error| format!("`{}`: {error}", self.manifest_path.display()))?,
            );
        }
        let config = load_local_config(&self.root)?;
        Ok(Project {
            root: self.root,
            manifest_path: self.manifest_path,
            manifest_source: self.manifest_source,
            manifest: self.manifest,
            config,
            name: package.name,
            version,
            entry: package.entry,
            source: package.source,
            dependencies,
        })
    }
}

/// A package: its manifest, where it was read from, and `[package]` normalized.
#[derive(Clone, Debug)]
pub struct Project {
    pub root: PathBuf,
    pub manifest_path: PathBuf,
    /// Kept verbatim because the build cache hashes the text, not the parse.
    pub manifest_source: String,
    pub manifest: Manifest,
    pub config: LocalConfig,
    pub name: String,
    pub version: Version,
    pub entry: Option<PathBuf>,
    pub source: Option<PathBuf>,
    /// `[dependencies]` with workspace inheritance applied.
    pub dependencies: BTreeMap<String, DependencySpec>,
}

impl Project {
    /// Root of the path-derived module tree.
    pub fn source_root(&self) -> Result<PathBuf, String> {
        let relative = self
            .source
            .clone()
            .or_else(|| {
                self.entry
                    .as_deref()
                    .and_then(Path::parent)
                    .map(std::path::Path::to_path_buf)
            })
            .unwrap_or_else(|| PathBuf::from("src"));
        let root = self.root.join(relative);
        root.canonicalize()
            .map_err(|error| format!("cannot read source root `{}`: {error}", root.display()))
    }

    /// The module a build of this package starts from.
    ///
    /// A library has none, and asking for one is a question about a package
    /// that cannot be answered rather than a path that happens not to exist.
    pub fn entry_path(&self) -> Result<PathBuf, String> {
        let entry = self.entry.as_ref().ok_or_else(|| {
            format!(
                "`{}` declares no `entry`; it is a library and has no module to start from",
                self.name
            )
        })?;
        Ok(self.root.join(entry))
    }

    /// `D-015`: a package entered through `lib.slp` is a library, and a library
    /// has no `main` to validate and no executable to link. A package that
    /// declares no `entry` at all is the same thing said more directly.
    pub fn is_library(&self) -> bool {
        match &self.entry {
            None => true,
            Some(entry) => entry.file_name().and_then(|name| name.to_str()) == Some("lib.slp"),
        }
    }
}

/// Package and dependency-alias names: ASCII letters, digits, `-`, `_`.
pub fn validate_package_name(name: &str) -> Result<(), String> {
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

/// Walk upwards from `start` looking for a manifest.
pub fn find_manifest(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|directory| directory.join(MANIFEST_FILE))
        .find(|candidate| candidate.is_file())
}

/// The manifest a command starts from: the one named, or the nearest above the
/// working directory.
pub fn starting_manifest(manifest_path: Option<PathBuf>) -> Result<PathBuf, String> {
    match manifest_path {
        Some(path) => Ok(path),
        None => {
            let current = std::env::current_dir()
                .map_err(|error| format!("cannot read the working directory: {error}"))?;
            find_manifest(&current).ok_or_else(|| {
                format!(
                    "no `{MANIFEST_FILE}` in `{}` or its parents",
                    current.display()
                )
            })
        }
    }
}

/// Read and parse one manifest, without looking at anything it references.
pub fn read_manifest(manifest_path: &Path) -> Result<RawManifest, String> {
    let manifest_path = manifest_path
        .canonicalize()
        .map_err(|error| format!("cannot read `{}`: {error}", manifest_path.display()))?;
    let root = manifest_path
        .parent()
        .ok_or_else(|| format!("`{}` has no parent directory", manifest_path.display()))?
        .to_path_buf();
    let manifest_source = fs::read_to_string(&manifest_path)
        .map_err(|error| format!("cannot read `{}`: {error}", manifest_path.display()))?;
    let manifest: Manifest = toml::from_str(&manifest_source)
        .map_err(|error| format!("cannot parse `{}`: {error}", manifest_path.display()))?;
    if manifest.package.is_none() && manifest.workspace.is_none() {
        return Err(format!(
            "`{}` defines neither `[package]` nor `[workspace]`",
            manifest_path.display()
        ));
    }
    Ok(RawManifest {
        root,
        manifest_path,
        manifest_source,
        manifest,
    })
}

/// Read `.slopium/config.toml` from a directory, or the defaults.
pub fn load_local_config(root: &Path) -> Result<LocalConfig, String> {
    let config_path = root.join(".slopium/config.toml");
    if !config_path.is_file() {
        return Ok(LocalConfig::default());
    }
    let source = fs::read_to_string(&config_path)
        .map_err(|error| format!("cannot read `{}`: {error}", config_path.display()))?;
    toml::from_str(&source)
        .map_err(|error| format!("cannot parse `{}`: {error}", config_path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(text: &str) -> Result<Manifest, String> {
        toml::from_str(text).map_err(|error| error.to_string())
    }

    #[test]
    fn parses_a_minimal_manifest() {
        let parsed = manifest(
            r#"
                [package]
                name = "hello"
                version = "0.1.0"
                entry = "src/main.slp"
            "#,
        )
        .unwrap();
        let package = parsed.package.unwrap();
        assert_eq!(package.name, "hello");
        assert_eq!(
            package.version.resolve("version", None).unwrap(),
            &Version::new(0, 1, 0)
        );
    }

    #[test]
    fn a_manifest_may_define_a_workspace_and_no_package() {
        let parsed = manifest(
            r#"
                [workspace]
                members = ["app", "helper"]
            "#,
        )
        .unwrap();
        assert!(parsed.package.is_none());
        assert_eq!(parsed.workspace.unwrap().members, ["app", "helper"]);
    }

    #[test]
    fn an_inherited_field_is_written_as_a_table() {
        let parsed = manifest(
            r#"
                [package]
                name = "hello"
                version.workspace = true
                entry = "src/main.slp"
            "#,
        )
        .unwrap();
        let version = Version::new(9, 9, 9);
        assert_eq!(
            parsed
                .package
                .unwrap()
                .version
                .resolve("version", Some(&version))
                .unwrap(),
            &version
        );
    }

    #[test]
    fn an_inherited_field_that_is_not_workspace_true_is_refused() {
        let error = manifest(
            r#"
                [package]
                name = "hello"
                version = { workspace = false }
                entry = "src/main.slp"
            "#,
        )
        .unwrap_err();
        assert!(error.contains("workspace = false"), "{error}");
    }

    #[test]
    fn rejects_a_version_that_is_not_a_version() {
        let error = manifest(
            r#"
                [package]
                name = "hello"
                version = "not-a-version"
                entry = "src/main.slp"
            "#,
        )
        .unwrap_err();
        assert!(error.contains("invalid version"), "{error}");
    }

    /// The old untagged enum answered "data did not match any variant"; a typo
    /// should name itself.
    #[test]
    fn a_misspelled_dependency_key_names_itself() {
        let error = manifest(
            r#"
                [package]
                name = "hello"
                version = "0.1.0"
                entry = "src/main.slp"

                [dependencies]
                geometry = { pth = "../geometry" }
            "#,
        )
        .unwrap_err();
        assert!(error.contains("pth"), "{error}");
    }

    #[test]
    fn dependency_sources_reject_meaningless_combinations() {
        let spec = DependencySpec {
            path: Some(PathBuf::from("../x")),
            toolchain: Some(true),
            ..DependencySpec::default()
        };
        assert!(spec.source("x").unwrap_err().contains("pick one"));

        let neither = DependencySpec::default();
        assert!(neither.source("x").unwrap_err().contains("names no source"));

        let disabled = DependencySpec {
            toolchain: Some(false),
            ..DependencySpec::default()
        };
        assert!(disabled
            .source("x")
            .unwrap_err()
            .contains("toolchain = false"));
    }

    #[test]
    fn a_git_dependency_names_one_commit() {
        let url = "https://example.invalid/geometry.git";
        let plain = DependencySpec {
            git: Some(url.to_owned()),
            ..DependencySpec::default()
        };
        assert_eq!(
            plain.source("geometry").unwrap(),
            SourceSpec::Git {
                url: url.to_owned(),
                reference: GitReference::DefaultBranch,
            }
        );

        let tagged = DependencySpec {
            git: Some(url.to_owned()),
            tag: Some("v1.4.0".to_owned()),
            ..DependencySpec::default()
        };
        assert_eq!(
            tagged.source("geometry").unwrap(),
            SourceSpec::Git {
                url: url.to_owned(),
                reference: GitReference::Tag("v1.4.0".to_owned()),
            }
        );

        let two = DependencySpec {
            git: Some(url.to_owned()),
            branch: Some("main".to_owned()),
            rev: Some("0123456".to_owned()),
            ..DependencySpec::default()
        };
        let error = two.source("geometry").unwrap_err();
        assert!(error.contains("one commit"), "{error}");
        assert!(error.contains("`branch`"), "{error}");
    }

    /// A commit of no repository is a typo with a plausible-looking key, so it
    /// says what is missing rather than "names no source".
    #[test]
    fn a_reference_without_a_repository_says_so() {
        let error = DependencySpec {
            branch: Some("main".to_owned()),
            ..DependencySpec::default()
        }
        .source("geometry")
        .unwrap_err();
        assert!(error.contains("without `git`"), "{error}");
    }

    #[test]
    fn a_git_dependency_that_is_also_a_path_is_refused() {
        let error = DependencySpec {
            git: Some("https://example.invalid/x.git".to_owned()),
            path: Some(PathBuf::from("../x")),
            ..DependencySpec::default()
        }
        .source("x")
        .unwrap_err();
        assert!(error.contains("pick one"), "{error}");
        assert!(error.contains("`git`"), "{error}");
    }

    #[test]
    fn an_absent_version_requirement_matches_anything() {
        assert_eq!(DependencySpec::default().requirement().to_string(), "*");
    }

    /// `D-053`: a bare requirement is the registry named `default`, which is
    /// still a name somebody has to configure.
    #[test]
    fn a_bare_requirement_is_the_default_registry() {
        let manifest: Manifest = toml::from_str(
            r#"
            [package]
            name = "demo"
            version = "0.1.0"

            [dependencies]
            geometry = "^1.2"
            physics = { version = "=2.0.0", registry = "internal" }
            "#,
        )
        .unwrap();

        let geometry = &manifest.dependencies["geometry"];
        assert_eq!(geometry.requirement().to_string(), "^1.2");
        assert_eq!(
            geometry.source("geometry").unwrap(),
            SourceSpec::Registry {
                registry: "default".to_owned(),
            }
        );
        assert_eq!(
            manifest.dependencies["physics"].source("physics").unwrap(),
            SourceSpec::Registry {
                registry: "internal".to_owned(),
            }
        );
    }

    /// The string form is read by a visitor rather than an untagged enum, so a
    /// mistyped key in the table form still names itself.
    #[test]
    fn a_mistyped_dependency_key_is_named() {
        let error = toml::from_str::<Manifest>(
            r#"
            [package]
            name = "demo"
            version = "0.1.0"

            [dependencies]
            geometry = { pth = "../geometry" }
            "#,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("pth"), "{error}");
    }

    #[test]
    fn a_registry_dependency_that_is_also_a_repository_is_refused() {
        let error = DependencySpec {
            git: Some("https://example.invalid/x.git".to_owned()),
            registry: Some("internal".to_owned()),
            ..DependencySpec::default()
        }
        .source("x")
        .unwrap_err();
        assert!(error.contains("pick one"), "{error}");
        assert!(error.contains("`registry`"), "{error}");
    }

    #[test]
    fn an_inherited_dependency_is_taken_whole_or_not_at_all() {
        let mut section = WorkspaceSection::default();
        section.dependencies.insert(
            "foundation".to_owned(),
            DependencySpec {
                path: Some(PathBuf::from("../foundation")),
                ..DependencySpec::default()
            },
        );
        let inheritance = Inheritance {
            section: &section,
            root: Path::new("/workspace"),
        };
        let inherited = DependencySpec {
            workspace: Some(true),
            ..DependencySpec::default()
        };
        // Rebased onto the workspace root, not the member's directory.
        assert_eq!(
            inherited
                .inherit("foundation", Some(inheritance))
                .unwrap()
                .path
                .as_deref(),
            Some(Path::new("/workspace/../foundation"))
        );

        let unknown = inherited.inherit("absent", Some(inheritance)).unwrap_err();
        assert!(unknown.contains("[workspace.dependencies]"), "{unknown}");

        let alone = inherited.inherit("foundation", None).unwrap_err();
        assert!(alone.contains("not in a workspace"), "{alone}");

        let half = DependencySpec {
            workspace: Some(true),
            version: Some(VersionReq::any()),
            ..DependencySpec::default()
        };
        assert!(half
            .inherit("foundation", Some(inheritance))
            .unwrap_err()
            .contains("taken whole"));
    }

    #[test]
    fn package_names_are_checked() {
        assert!(validate_package_name("path-dependencies").is_ok());
        assert!(validate_package_name("a_b9").is_ok());
        assert!(validate_package_name("").is_err());
        assert!(validate_package_name("has space").is_err());
        assert!(validate_package_name("colon:name").is_err());
    }
}
