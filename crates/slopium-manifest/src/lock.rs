//! `Slopium.lock` — the resolved graph, written down.
//!
//! The file is rendered by hand rather than serialized, because its whole value
//! is that it diffs cleanly: a fixed field order, packages sorted by name, and
//! no table reordering when a dependency is added. It records paths relative to
//! itself, so a checkout at a different absolute path locks identically.
//!
//! A package whose bytes cannot change under the lock records a `checksum` —
//! the digest of its archive (`D-039`), which is what a vendored or stored copy
//! is checked against before anything reads it. A path dependency records none:
//! it is a working tree, and hashing one would rewrite the lock on every
//! keystroke.

use crate::resolve::ResolvedPackage;
use crate::sha256::Digest;
use crate::source::SourceId;
use crate::version::Version;
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

/// The name of the lockfile, everywhere it is looked for.
pub const LOCK_FILE: &str = "Slopium.lock";

/// The lockfile format version, bumped when the shape of the file changes.
/// Version 2 added `checksum`.
pub const LOCK_FORMAT: u32 = 2;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LockedPackage {
    pub name: String,
    pub version: Version,
    pub source: String,
    pub dependencies: Vec<String>,
    /// Absent for a working tree, which has no bytes to pin.
    pub checksum: Option<Digest>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Lockfile {
    pub format: u32,
    /// Sorted by name; a name is unique in a resolved graph (`D-036`).
    pub packages: Vec<LockedPackage>,
}

impl Lockfile {
    /// Describe a resolved graph, with path sources written relative to `base`.
    ///
    /// The graph is the whole workspace's, not one package's: members share a
    /// lock, so building one member must not rewrite what another recorded.
    pub fn from_packages(
        packages: &std::collections::BTreeMap<String, ResolvedPackage>,
        base: &Path,
    ) -> Self {
        let mut packages = packages
            .values()
            .map(|package| LockedPackage {
                name: package.id.name.clone(),
                version: package.id.version.clone(),
                source: match &package.id.source {
                    SourceId::Path(path) => {
                        format!("path+{}", render_path(&relative_to(base, path)))
                    }
                    other => other.to_lock_field(),
                },
                dependencies: {
                    let mut names = package.dependencies.clone();
                    names.sort();
                    names.dedup();
                    names
                },
                checksum: package.checksum,
            })
            .collect::<Vec<_>>();
        packages.sort_by(|left, right| left.name.cmp(&right.name));
        Self {
            format: LOCK_FORMAT,
            packages,
        }
    }

    pub fn render(&self) -> String {
        let mut output = String::new();
        let _ = writeln!(
            output,
            "# Written by slopium. Do not edit by hand.\nversion = {}",
            self.format
        );
        for package in &self.packages {
            let _ = write!(
                output,
                "\n[[package]]\nname = {}\nversion = {}\nsource = {}\n",
                quote(&package.name),
                quote(&package.version.to_string()),
                quote(&package.source),
            );
            if let Some(checksum) = &package.checksum {
                let _ = writeln!(output, "checksum = {}", quote(&checksum.to_string()));
            }
            if package.dependencies.is_empty() {
                let _ = writeln!(output, "dependencies = []");
            } else {
                let _ = writeln!(output, "dependencies = [");
                for dependency in &package.dependencies {
                    let _ = writeln!(output, "    {},", quote(dependency));
                }
                let _ = writeln!(output, "]");
            }
        }
        output
    }

    /// Parse a lockfile. Anything the current format does not understand is an
    /// error rather than something to guess at — a lock that is half believed
    /// is worse than no lock.
    pub fn parse(text: &str) -> Result<Self, String> {
        let document: toml::Value =
            toml::from_str(text).map_err(|error| format!("cannot parse `{LOCK_FILE}`: {error}"))?;
        let format = document
            .get("version")
            .and_then(toml::Value::as_integer)
            .ok_or_else(|| format!("`{LOCK_FILE}` has no `version` field"))?;
        if format != i64::from(LOCK_FORMAT) {
            return Err(format!(
                "`{LOCK_FILE}` is version {format} and this slopium writes version {LOCK_FORMAT}"
            ));
        }

        let mut packages = Vec::new();
        for entry in document
            .get("package")
            .and_then(toml::Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let field = |name: &str| -> Result<String, String> {
                entry
                    .get(name)
                    .and_then(toml::Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| format!("`{LOCK_FILE}` has a package without `{name}`"))
            };
            let name = field("name")?;
            let version = Version::parse(&field("version")?)?;
            let source = field("source")?;
            SourceId::from_lock_field(&source)
                .map_err(|error| format!("`{LOCK_FILE}`: package `{name}`: {error}"))?;
            let dependencies = entry
                .get("dependencies")
                .and_then(toml::Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(toml::Value::as_str)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let checksum = match entry.get("checksum") {
                Some(value) => Some(
                    value
                        .as_str()
                        .ok_or_else(|| {
                            format!("`{LOCK_FILE}`: package `{name}` has a non-string `checksum`")
                        })
                        .and_then(|text| {
                            Digest::parse(text).map_err(|error| {
                                format!("`{LOCK_FILE}`: package `{name}`: {error}")
                            })
                        })?,
                ),
                None => None,
            };
            packages.push(LockedPackage {
                name,
                version,
                source,
                dependencies,
                checksum,
            });
        }
        packages.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(Self {
            format: LOCK_FORMAT,
            packages,
        })
    }
}

fn quote(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Render a path with forward slashes, so a lock is the same text everywhere.
fn render_path(path: &Path) -> String {
    let rendered = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    if rendered.is_empty() {
        ".".to_owned()
    } else {
        rendered
    }
}

/// `path` expressed relative to `base`, falling back to the absolute path when
/// the two share no prefix — a dependency on another filesystem is unusual but
/// not an error.
fn relative_to(base: &Path, path: &Path) -> PathBuf {
    let normalize = |value: &Path| -> Vec<String> {
        value
            .components()
            .filter(|component| !matches!(component, Component::CurDir))
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect()
    };
    let base_parts = normalize(base);
    let path_parts = normalize(path);
    if base.is_absolute() != path.is_absolute() {
        return path.to_path_buf();
    }
    let shared = base_parts
        .iter()
        .zip(path_parts.iter())
        .take_while(|(left, right)| left == right)
        .count();
    if shared == 0 && path.is_absolute() {
        return path.to_path_buf();
    }
    let mut relative = PathBuf::new();
    for _ in shared..base_parts.len() {
        relative.push("..");
    }
    for part in &path_parts[shared..] {
        relative.push(part);
    }
    relative
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(name: &str, version: &str, source: &str, dependencies: &[&str]) -> LockedPackage {
        LockedPackage {
            name: name.to_owned(),
            version: Version::parse(version).unwrap(),
            source: source.to_owned(),
            dependencies: dependencies.iter().map(|name| (*name).to_owned()).collect(),
            checksum: (source == "toolchain").then(|| crate::sha256::sha256(name.as_bytes())),
        }
    }

    fn sample() -> Lockfile {
        Lockfile {
            format: LOCK_FORMAT,
            packages: vec![
                package("application", "1.0.0", "path+.", &["mathlib", "std"]),
                package("foundation", "1.2.0", "path+../foundation", &[]),
                package("mathlib", "1.0.0", "path+../mathlib", &["foundation"]),
                package("std", "0.3.7", "toolchain", &[]),
            ],
        }
    }

    #[test]
    fn rendering_round_trips() {
        let lock = sample();
        assert_eq!(Lockfile::parse(&lock.render()).unwrap(), lock);
    }

    #[test]
    fn rendering_is_stable() {
        let lock = sample();
        assert_eq!(lock.render(), lock.render());
        assert!(lock.render().starts_with("# Written by slopium."));
    }

    /// The point of the file is that it diffs cleanly, so the exact bytes are
    /// part of the contract.
    #[test]
    fn the_rendered_shape_is_the_documented_one() {
        let lock = Lockfile {
            format: LOCK_FORMAT,
            packages: vec![package("std", "0.3.7", "toolchain", &[])],
        };
        assert_eq!(
            lock.render(),
            "# Written by slopium. Do not edit by hand.\nversion = 2\n\n[[package]]\nname = \"std\"\nversion = \"0.3.7\"\nsource = \"toolchain\"\nchecksum = \"a7f5397443359ea76c50be82c77f1f893a060925b51a332cc5da906f83d3344e\"\ndependencies = []\n"
        );
    }

    #[test]
    fn packages_are_sorted_however_they_arrive() {
        let text = sample().render();
        let reversed = Lockfile {
            format: LOCK_FORMAT,
            packages: sample().packages.into_iter().rev().collect(),
        };
        assert_eq!(Lockfile::parse(&reversed.render()).unwrap().render(), text);
    }

    #[test]
    fn a_future_format_is_refused_rather_than_guessed_at() {
        let error = Lockfile::parse("version = 99\n").unwrap_err();
        assert!(error.contains("version 99"), "{error}");
    }

    #[test]
    fn an_unknown_source_is_refused() {
        let error = Lockfile::parse(
            "version = 2\n\n[[package]]\nname = \"x\"\nversion = \"1.0.0\"\nsource = \"packages+https://example\"\ndependencies = []\n",
        )
        .unwrap_err();
        assert!(error.contains("unknown package source"), "{error}");
    }

    #[test]
    fn paths_are_written_relative_to_the_lock() {
        assert_eq!(
            render_path(&relative_to(
                Path::new("/home/x/dev/app"),
                Path::new("/home/x/dev/mathlib")
            )),
            "../mathlib"
        );
        assert_eq!(
            render_path(&relative_to(
                Path::new("/home/x/dev/app"),
                Path::new("/home/x/dev/app")
            )),
            "."
        );
        assert_eq!(
            render_path(&relative_to(
                Path::new("/home/x/dev/app"),
                Path::new("/home/x/dev/app/vendor/thing")
            )),
            "vendor/thing"
        );
    }
}
