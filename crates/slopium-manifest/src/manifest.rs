//! `Slopium.toml`, parsed in one place (`D-034`).
//!
//! This used to be two structs: `Manifest` in the project manager and a smaller
//! `WorkspaceManifest` in the language server, which parsed a subset of the same
//! file and walked the dependency graph a second time. Anything both of them
//! must agree about belongs outside both (`D-025`).

use crate::source::SourceSpec;
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
    pub entry: PathBuf,
    pub source: Option<PathBuf>,
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
/// table matched no variant instead of naming the typo.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DependencySpec {
    pub path: Option<PathBuf>,
    pub toolchain: Option<bool>,
    pub version: Option<VersionReq>,
    /// `workspace = true`: take this entry from `[workspace.dependencies]`.
    pub workspace: Option<bool>,
}

impl DependencySpec {
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
                if self.path.is_some() || self.toolchain.is_some() || self.version.is_some() {
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
    /// meaning. The `Git` and `Registry` arms arrive in v0.4.3 and v0.4.4.
    pub fn source(&self, name: &str) -> Result<SourceSpec, String> {
        if self.workspace.is_some() {
            return Err(format!(
                "dependency `{name}` still says `workspace = true` after inheritance"
            ));
        }
        match (&self.path, self.toolchain) {
            (Some(_), Some(_)) => Err(format!(
                "dependency `{name}` names both `path` and `toolchain`; pick one"
            )),
            (Some(path), None) => Ok(SourceSpec::Path(path.clone())),
            (None, Some(true)) => Ok(SourceSpec::Toolchain),
            (None, Some(false)) => Err(format!(
                "dependency `{name}` has `toolchain = false`; name a `path` instead"
            )),
            (None, None) => Err(format!(
                "dependency `{name}` names no source; add `path` or `toolchain = true`"
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
    pub entry: PathBuf,
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
            .or_else(|| self.entry.parent().map(std::path::Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("src"));
        let root = self.root.join(relative);
        root.canonicalize()
            .map_err(|error| format!("cannot read source root `{}`: {error}", root.display()))
    }

    pub fn entry_path(&self) -> PathBuf {
        self.root.join(&self.entry)
    }

    /// `D-015`: a package entered through `lib.slp` is a library, and a library
    /// has no `main` to validate and no executable to link.
    pub fn is_library(&self) -> bool {
        self.entry.file_name().and_then(|name| name.to_str()) == Some("lib.slp")
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

fn load_local_config(root: &Path) -> Result<LocalConfig, String> {
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
    fn an_absent_version_requirement_matches_anything() {
        assert_eq!(DependencySpec::default().requirement().to_string(), "*");
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
