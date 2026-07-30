//! `Slopium.toml`, parsed in one place (`D-034`).
//!
//! This used to be two structs: `Manifest` in the project manager and a smaller
//! `WorkspaceManifest` in the language server, which parsed a subset of the same
//! file and walked the dependency graph a second time. Anything both of them
//! must agree about belongs outside both (`D-025`).

use crate::source::SourceSpec;
use crate::version::{Version, VersionReq};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

/// The name of a manifest file, everywhere it is looked for.
pub const MANIFEST_FILE: &str = "Slopium.toml";

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub package: Package,
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
    pub version: Version,
    pub entry: PathBuf,
    pub source: Option<PathBuf>,
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
}

impl DependencySpec {
    /// Which source this entry names, rejecting the combinations that have no
    /// meaning. The `Git` and `Registry` arms arrive in v0.4.3 and v0.4.4.
    pub fn source(&self, name: &str) -> Result<SourceSpec, String> {
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

/// A manifest together with where it was read from.
#[derive(Clone, Debug)]
pub struct Project {
    pub root: PathBuf,
    pub manifest_path: PathBuf,
    /// Kept verbatim because the build cache hashes the text, not the parse.
    pub manifest_source: String,
    pub manifest: Manifest,
    pub config: LocalConfig,
}

impl Project {
    pub fn name(&self) -> &str {
        &self.manifest.package.name
    }

    pub fn version(&self) -> &Version {
        &self.manifest.package.version
    }

    /// Root of the path-derived module tree.
    pub fn source_root(&self) -> Result<PathBuf, String> {
        let relative = self
            .manifest
            .package
            .source
            .clone()
            .or_else(|| {
                self.manifest
                    .package
                    .entry
                    .parent()
                    .map(std::path::Path::to_path_buf)
            })
            .unwrap_or_else(|| PathBuf::from("src"));
        let root = self.root.join(relative);
        root.canonicalize()
            .map_err(|error| format!("cannot read source root `{}`: {error}", root.display()))
    }

    pub fn entry_path(&self) -> PathBuf {
        self.root.join(&self.manifest.package.entry)
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

/// Read and validate a manifest.
pub fn load_project(manifest_path: Option<PathBuf>) -> Result<Project, String> {
    let manifest_path = match manifest_path {
        Some(path) => path,
        None => {
            let current = std::env::current_dir()
                .map_err(|error| format!("cannot read the working directory: {error}"))?;
            find_manifest(&current).ok_or_else(|| {
                format!(
                    "no `{MANIFEST_FILE}` in `{}` or its parents",
                    current.display()
                )
            })?
        }
    };
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
    validate_package_name(&manifest.package.name)?;

    let config_path = root.join(".slopium/config.toml");
    let config = if config_path.is_file() {
        let source = fs::read_to_string(&config_path)
            .map_err(|error| format!("cannot read `{}`: {error}", config_path.display()))?;
        toml::from_str(&source)
            .map_err(|error| format!("cannot parse `{}`: {error}", config_path.display()))?
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
        assert_eq!(parsed.package.name, "hello");
        assert_eq!(parsed.package.version, Version::new(0, 1, 0));
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
            version: None,
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
    fn package_names_are_checked() {
        assert!(validate_package_name("path-dependencies").is_ok());
        assert!(validate_package_name("a_b9").is_ok());
        assert!(validate_package_name("").is_err());
        assert!(validate_package_name("has space").is_err());
        assert!(validate_package_name("colon:name").is_err());
    }
}
