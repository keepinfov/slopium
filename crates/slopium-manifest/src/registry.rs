//! Reading a registry index (`D-052`).
//!
//! A registry is a directory a file server serves: an `index/` tree of one file
//! per package, each holding one JSON object per line per published version,
//! and a `packages/` tree of the archives those lines describe. There is no
//! protocol beyond "fetch this path", which is why any file server — or a
//! directory, or `file://` — is a registry and no server lives in this
//! repository.
//!
//! What the index is trusted for is stated in `D-055`: it makes resolution fast
//! by carrying the requirements of versions that have not been downloaded, and
//! it is trusted for nothing else. Every byte it points at is checked against a
//! digest before anything reads it.
//!
//! Index files fetched over the network are kept under
//! `$SLOPIUM_HOME/index/<digest of the index url>/`, which is what lets
//! `--offline` resolve a dependency the lock does not already pin. The cache is
//! a fallback and never a shortcut: an online run always fetches and always
//! overwrites, because an index that grew a version is the whole reason to read
//! one, and serving a stale copy would pin an old version silently. A registry
//! that is a directory needs none of this — it is already local, and reading it
//! was never a network operation.

use crate::manifest::{validate_package_name, LocalConfig};
use crate::sha256::{sha256, Digest};
use crate::signature::{PublicKey, Signature, SIGNATURE_EXTENSION};
use crate::store::Access;
use crate::version::{Version, VersionReq};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

/// The subdirectory of a registry holding one file per package.
pub const INDEX_DIRECTORY: &str = "index";
/// The subdirectory of a registry holding the archives themselves.
pub const PACKAGES_DIRECTORY: &str = "packages";

/// Where a dependency in an index entry is taken from.
///
/// `D-054`: naming nothing means the index the entry itself came from, never
/// the consumer's default registry. A package published to an internal index
/// cannot be made to reach a public one by how a consumer is configured.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IndexSource {
    /// The index this entry was read from.
    SameIndex,
    /// The library bundled with the compiler, which is in no index.
    Toolchain,
}

/// One requirement of one published version.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexDependency {
    pub name: String,
    pub requirement: VersionReq,
    pub source: IndexSource,
}

/// One published version: one line of one index file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IndexEntry {
    pub name: String,
    pub version: Version,
    pub dependencies: Vec<IndexDependency>,
    /// What `packages/<name>/<name>-<version>.sl.tar` must hash to.
    pub checksum: Digest,
    /// A yanked version is not selected, but is still built when a lock
    /// already names it (`D-055`).
    pub yanked: bool,
    /// Who signed this version, if anybody did. The same line is written to
    /// `<archive>.sig` beside the archive, so bytes on a disk carry their own
    /// signature and an index alone is still a complete published record;
    /// neither copy is trusted, because both are checked against the same
    /// statement and the same key list (`D-056`).
    pub signature: Option<Signature>,
}

/// The wire form. Unknown fields are ignored on purpose: an index that grows a
/// field must not stop older clients from reading it (`D-052`).
#[derive(Deserialize, Serialize)]
struct RawEntry {
    name: String,
    version: String,
    #[serde(default)]
    dependencies: Vec<RawDependency>,
    checksum: String,
    #[serde(default)]
    yanked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    signature: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct RawDependency {
    name: String,
    requirement: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<String>,
}

impl IndexEntry {
    /// This entry as the one line an index file holds it on.
    pub fn render(&self) -> Result<String, String> {
        let raw = RawEntry {
            name: self.name.clone(),
            version: self.version.to_string(),
            dependencies: self
                .dependencies
                .iter()
                .map(|dependency| RawDependency {
                    name: dependency.name.clone(),
                    requirement: dependency.requirement.to_string(),
                    source: match &dependency.source {
                        IndexSource::SameIndex => None,
                        IndexSource::Toolchain => Some("toolchain".to_owned()),
                    },
                })
                .collect(),
            checksum: self.checksum.to_string(),
            yanked: self.yanked,
            signature: self.signature.map(|signature| signature.to_string()),
        };
        serde_json::to_string(&raw)
            .map_err(|error| format!("cannot render an index entry: {error}"))
    }

    fn parse(line: &str, file: &str) -> Result<Self, String> {
        let raw: RawEntry = serde_json::from_str(line).map_err(|error| {
            format!("SL1036: `{file}` holds a line that is not an index entry: {error}")
        })?;
        let malformed = |what: String| format!("SL1036: `{file}`: {what}");
        let mut dependencies = Vec::new();
        for dependency in raw.dependencies {
            dependencies.push(IndexDependency {
                requirement: VersionReq::parse(&dependency.requirement).map_err(malformed)?,
                source: match dependency.source.as_deref() {
                    None => IndexSource::SameIndex,
                    Some("toolchain") => IndexSource::Toolchain,
                    // A published package's manifest cannot name another
                    // registry either (`D-054`), so an entry that does is
                    // describing something this release cannot follow.
                    Some(other) => {
                        return Err(malformed(format!(
                            "`{}` is taken from `{other}`, and a published package depends only on its own registry and the toolchain in this release",
                            dependency.name
                        )))
                    }
                },
                name: dependency.name,
            });
        }
        Ok(Self {
            version: Version::parse(&raw.version).map_err(malformed)?,
            checksum: Digest::parse(&raw.checksum).map_err(malformed)?,
            signature: raw
                .signature
                .as_deref()
                .map(Signature::parse)
                .transpose()
                .map_err(malformed)?,
            name: raw.name,
            dependencies,
            yanked: raw.yanked,
        })
    }
}

/// The path of a package's index file, relative to `index/`.
///
/// Fanned out by name length so a large index is a tree rather than one
/// directory of a hundred thousand files.
pub fn index_path(name: &str) -> Result<String, String> {
    validate_package_name(name)?;
    Ok(match name.len() {
        1 => format!("1/{name}.json"),
        2 => format!("2/{name}.json"),
        3 => format!("3/{}/{name}.json", &name[..1]),
        _ => format!("{}/{}/{name}.json", &name[..2], &name[2..4]),
    })
}

/// The path of a package's archive, relative to the registry root.
pub fn archive_path(name: &str, version: &Version) -> String {
    format!(
        "{PACKAGES_DIRECTORY}/{name}/{name}-{version}.{}",
        crate::archive::ARCHIVE_EXTENSION
    )
}

/// The path of a package's detached signature, relative to the registry root.
pub fn signature_path(name: &str, version: &Version) -> String {
    format!("{}.{SIGNATURE_EXTENSION}", archive_path(name, version))
}

/// An index URL with the trailing slash that means nothing removed, so two
/// spellings of one registry are one source id.
fn normalize_index(index: &str) -> String {
    index.trim_end_matches('/').to_owned()
}

/// One configured registry, and how to reach it.
#[derive(Clone, Debug)]
pub struct Registry {
    /// What `.slopium/config.toml` calls it, for messages only.
    name: String,
    /// The URL that identifies it in a lockfile (`D-052`).
    index: String,
    /// Who this checkout will accept packages from here. Empty means signatures
    /// are not checked (`D-057`).
    trusted: Vec<PublicKey>,
    transport: Transport,
    /// Index files already read, so backtracking does not re-fetch. An empty
    /// entry is a package the registry does not have.
    cached: Arc<Mutex<BTreeMap<String, Arc<Vec<IndexEntry>>>>>,
    /// Whether this run may reach the network at all.
    access: Access,
    /// Where fetched index files are kept between runs, if anywhere. `None` is
    /// a registry nobody gave a store to — the tests, and the LSP, which
    /// resolves nothing.
    cache: Option<PathBuf>,
}

#[derive(Clone, Debug)]
enum Transport {
    /// A directory on this machine, from `file://` or a plain relative path.
    Directory(PathBuf),
    /// Fetched with `curl` (`D-037`).
    Url,
}

impl Registry {
    /// `root` is what a configured relative path is written against — the
    /// workspace root, since the configuration belongs to the checkout.
    pub fn new(name: &str, index: &str, root: &Path) -> Result<Self, String> {
        Self::trusting(name, index, root, &[])
    }

    /// The same, with the keys `[registry.<name>] trusted-keys` listed.
    pub fn trusting(
        name: &str,
        index: &str,
        root: &Path,
        trusted: &[String],
    ) -> Result<Self, String> {
        let index = normalize_index(index);
        let transport = match index.split_once("://") {
            None => Transport::Directory(root.join(&index)),
            Some(("file", rest)) => Transport::Directory(file_url_path(&index, rest)?),
            Some(("https", _)) => Transport::Url,
            // `D-052`: whoever answers a plaintext index chooses what a first
            // resolution pins, and the checksum that would catch tampered bytes
            // came from the index too. Loopback is the one hop with nothing in
            // between, and it is what the transport is tested over.
            Some(("http", rest)) if is_loopback(rest) => Transport::Url,
            Some(("http", _)) => {
                return Err(format!(
                    "SL1030: registry `{name}` has the plaintext index `{index}`; use `https://`, or `http://` only for a loopback address"
                ))
            }
            Some((scheme, _)) => {
                return Err(format!(
                    "SL1030: registry `{name}` has the index `{index}`, and `{scheme}` is not a transport this toolchain has"
                ))
            }
        };
        let keys = trusted
            .iter()
            .map(|key| {
                PublicKey::parse(key).map_err(|error| {
                    format!(
                        "`[registry.{name}] trusted-keys` holds something that is not one: {error}"
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            name: name.to_owned(),
            index,
            trusted: keys,
            transport,
            cached: Arc::new(Mutex::new(BTreeMap::new())),
            access: Access::Online,
            cache: None,
        })
    }

    /// Tell this registry what this run is allowed to do and where it may keep
    /// what it fetched.
    ///
    /// Set from `Sources`, which owns both the access policy and the store, so
    /// that a registry cannot end up online while the rest of the run is
    /// offline. Nothing else calls it.
    pub(crate) fn serve(&mut self, access: Access, store_root: &Path) {
        self.access = access;
        self.cache = Some(
            store_root
                .join("index")
                .join(sha256(self.index.as_bytes()).to_string()),
        );
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Who this checkout accepts packages from here. Empty means it accepts
    /// them unsigned, which is a state somebody chose by not choosing.
    pub fn trusted_keys(&self) -> &[PublicKey] {
        &self.trusted
    }

    /// The directory this registry is, if it is one.
    ///
    /// Only a directory can be published to. There is no upload protocol
    /// because there is no server, and inventing one to reach an `https://`
    /// index would be inventing the server too (`D-059`).
    pub fn directory(&self) -> Option<&Path> {
        match &self.transport {
            Transport::Directory(root) => Some(root),
            Transport::Url => None,
        }
    }

    /// The URL this registry is identified by in a lockfile.
    pub fn index(&self) -> &str {
        &self.index
    }

    /// Every published version of a package, oldest first, or an empty list if
    /// this registry does not have the package at all.
    pub fn versions(&self, name: &str) -> Result<Arc<Vec<IndexEntry>>, String> {
        if let Some(cached) = self.cached.lock().unwrap().get(name) {
            return Ok(cached.clone());
        }
        let within_index = index_path(name)?;
        let relative = format!("{INDEX_DIRECTORY}/{within_index}");
        let file = format!("{}/{relative}", self.index);
        let cached_at = self.cache.as_ref().map(|root| root.join(&within_index));
        let mut entries = Vec::new();
        if let Some(bytes) = self.index_file(name, cached_at.as_deref(), &relative)? {
            let text = String::from_utf8(bytes)
                .map_err(|_| format!("SL1036: `{file}` is not UTF-8, so it is not an index"))?;
            for line in text.lines().filter(|line| !line.trim().is_empty()) {
                let entry = IndexEntry::parse(line, &file)?;
                if entry.name != name {
                    return Err(format!(
                        "SL1036: `{file}` holds an entry for `{}`, but it is the index file of `{name}`",
                        entry.name
                    ));
                }
                entries.push(entry);
            }
            entries.sort_by(|left, right| left.version.cmp(&right.version));
        }
        let entries = Arc::new(entries);
        self.cached
            .lock()
            .unwrap()
            .insert(name.to_owned(), entries.clone());
        Ok(entries)
    }

    /// The bytes of a published archive.
    pub fn archive(&self, name: &str, version: &Version) -> Result<Vec<u8>, String> {
        let relative = archive_path(name, version);
        self.read(&relative)?.ok_or_else(|| {
            format!(
                "SL1037: registry `{}` has no `{relative}`, though its index publishes {name} v{version}",
                self.name
            )
        })
    }

    /// Who signed a published version, if anybody did.
    ///
    /// The `.sig` beside the archive is asked first, because it is the copy an
    /// archive carries with it and the one a store keeps. An index that
    /// publishes the signature and no `.sig` still answers — the two hold the
    /// same line, and neither is believed on its own (`D-056`).
    pub fn signature(&self, name: &str, version: &Version) -> Result<Option<Signature>, String> {
        let relative = signature_path(name, version);
        if let Some(bytes) = self.read(&relative)? {
            let text = String::from_utf8(bytes)
                .map_err(|_| format!("`{relative}` is not text, so it is not a signature"))?;
            return Signature::parse(text.trim())
                .map(Some)
                .map_err(|error| format!("SL1036: `{relative}`: {error}"));
        }
        Ok(self
            .versions(name)?
            .iter()
            .find(|entry| entry.version == *version)
            .and_then(|entry| entry.signature))
    }

    /// One package's index file, from wherever this run is allowed to read it.
    ///
    /// The three cases are different enough to be worth naming. A directory
    /// registry is read directly, offline or not, because reading a directory
    /// was never a network operation. An online run fetches and writes what it
    /// got through to the cache. An offline run reads the cache and nothing
    /// else, which is what makes resolving an unpinned dependency possible at
    /// all without a network.
    fn index_file(
        &self,
        name: &str,
        cached_at: Option<&Path>,
        relative: &str,
    ) -> Result<Option<Vec<u8>>, String> {
        if matches!(self.transport, Transport::Directory(_)) {
            return self.read(relative);
        }
        if self.access == Access::Offline {
            let Some(path) = cached_at else {
                return Err(format!(
                    "SL1011: `--offline` forbids reading the index of `{}`, and this run has no index cache to read instead",
                    self.index
                ));
            };
            return match std::fs::read(path) {
                Ok(bytes) => Ok(Some(bytes)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(format!(
                    "SL1011: `--offline` forbids reading the index of `{}`, and no copy of `{name}`'s index file is cached at `{}`. Resolve once without `--offline` to put one there",
                    self.index,
                    path.display()
                )),
                // Any other failure to read is the operating system's to
                // explain: the copy is there, nothing somebody wrote or asked
                // for is wrong, so no code fronts the message (`D-071`).
                Err(error) => Err(format!("cannot read `{}`: {error}", path.display())),
            };
        }
        let fetched = self.read(relative)?;
        if let Some(path) = cached_at {
            match &fetched {
                Some(bytes) => self.cache_index_file(path, bytes),
                // Online the index is the authority, so a package it has
                // stopped serving stops resolving here too. Leaving the old
                // copy would make `--offline` disagree with the last online
                // run, which is the one thing a cache must never do.
                None => {
                    let _ = std::fs::remove_file(path);
                }
            }
        }
        Ok(fetched)
    }

    /// Keep a fetched index file for the next run, best effort.
    ///
    /// A cache write that fails is not a build failure: the resolution it
    /// belongs to has already succeeded, and refusing it would turn a full disk
    /// into a broken toolchain. Losing the write costs one fetch next time.
    ///
    /// The `url` file at the top is for whoever opens `$SLOPIUM_HOME` and
    /// wonders what a directory named after a digest holds.
    fn cache_index_file(&self, path: &Path, bytes: &[u8]) {
        let (Some(root), Some(directory)) = (self.cache.as_deref(), path.parent()) else {
            return;
        };
        if std::fs::create_dir_all(directory).is_err() {
            return;
        }
        let temporary = directory.join(format!(".index-{}", crate::store::scratch_suffix()));
        if std::fs::write(&temporary, bytes).is_err() || std::fs::rename(&temporary, path).is_err()
        {
            let _ = std::fs::remove_file(&temporary);
            return;
        }
        let marker = root.join("url");
        if !marker.exists() {
            let _ = std::fs::write(marker, format!("{}\n", self.index));
        }
    }

    /// One file of this registry, or `None` if the registry does not have it.
    fn read(&self, relative: &str) -> Result<Option<Vec<u8>>, String> {
        match &self.transport {
            Transport::Directory(root) => match std::fs::read(root.join(relative)) {
                Ok(bytes) => Ok(Some(bytes)),
                // A file that is not there means the registry does not publish
                // it — but only if the registry is there at all. A directory
                // that does not exist is a misconfigured or moved registry, and
                // answering "it publishes nothing" would send whoever reads the
                // message looking for a package instead of for a path.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    if root.is_dir() {
                        Ok(None)
                    } else {
                        Err(format!(
                            "SL1030: registry `{}` is the directory `{}`, and there is no such directory",
                            self.name,
                            root.display()
                        ))
                    }
                }
                Err(error) => Err(format!(
                    "SL1037: cannot read `{}` from registry `{}`: {error}",
                    root.join(relative).display(),
                    self.name
                )),
            },
            Transport::Url => self.fetch(&format!("{}/{relative}", self.index)),
        }
    }

    /// `curl` with an argument list nothing can extend (`D-037`).
    ///
    /// The body goes to a file and the status code to stdout, because a client
    /// that cannot tell "this package is not published here" from "the server
    /// is broken" would turn an outage into a resolution error.
    fn fetch(&self, url: &str) -> Result<Option<Vec<u8>>, String> {
        // Index reads have already gone through `index_file`, which serves the
        // cache offline. Anything else reaching here offline — an archive, a
        // detached signature — is a download, and `Sources` refuses those
        // before asking; this is the backstop that keeps a stray path from
        // running `curl` behind `--offline`.
        if self.access == Access::Offline {
            return Err(format!(
                "SL1011: `--offline` forbids fetching `{url}` from registry `{}`",
                self.name
            ));
        }
        let body = std::env::temp_dir().join(format!(
            "slopium-fetch-{}-{}",
            std::process::id(),
            crate::sha256::sha256(url.as_bytes())
        ));
        let output = Command::new("curl")
            .args([
                "--silent",
                "--show-error",
                "--location",
                "--max-redirs",
                "5",
                "--max-time",
                "120",
                "--proto",
                "=http,https",
                "--write-out",
                "%{http_code}",
                "--output",
            ])
            .arg(&body)
            .arg(url)
            .output();
        let result = self.interpret(url, output, &body);
        let _ = std::fs::remove_file(&body);
        result
    }

    fn interpret(
        &self,
        url: &str,
        output: std::io::Result<std::process::Output>,
        body: &Path,
    ) -> Result<Option<Vec<u8>>, String> {
        let output = match output {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(
                    "SL1037: `curl` is not on PATH, and it is how this toolchain fetches over the network"
                        .to_owned(),
                )
            }
            Err(error) => return Err(format!("SL1037: cannot run `curl`: {error}")),
        };
        if !output.status.success() {
            return Err(format!(
                "SL1037: cannot fetch `{url}`: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let code = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        match code.as_str() {
            "200" => Ok(Some(std::fs::read(body).map_err(|error| {
                format!("SL1037: cannot read what was fetched from `{url}`: {error}")
            })?)),
            "404" | "410" => Ok(None),
            other => Err(format!(
                "SL1037: registry `{}` answered `{other}` for `{url}`",
                self.name
            )),
        }
    }
}

/// The path a `file://` URL names.
fn file_url_path(url: &str, rest: &str) -> Result<PathBuf, String> {
    let path = match rest.split_once('/') {
        Some(("", path)) => path,
        Some((host, _)) => {
            return Err(format!(
                "registry index `{url}` names the host `{host}`; a `file://` index is a path on this machine"
            ))
        }
        None => {
            return Err(format!(
                "registry index `{url}` names no path"
            ))
        }
    };
    if path.contains('%') {
        return Err(format!(
            "registry index `{url}` is percent-encoded; write the path itself"
        ));
    }
    Ok(PathBuf::from(format!("/{path}")))
}

fn is_loopback(rest: &str) -> bool {
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default()
        .rsplit_once(':')
        .map(|(host, _)| host)
        .unwrap_or(rest.split(['/', '?', '#']).next().unwrap_or_default());
    matches!(host, "localhost" | "127.0.0.1" | "[::1]")
}

/// Every registry this checkout has been told about.
///
/// There is no built-in default (`D-053`), so an unconfigured name is an error
/// rather than a download from somewhere nobody chose.
#[derive(Clone, Debug, Default)]
pub struct Registries {
    by_name: BTreeMap<String, Registry>,
}

impl Registries {
    /// Tell every registry what this run may do and where it may cache.
    ///
    /// Called from `Sources`, which holds the access policy and the store
    /// already. Doing it in one place is the point: a registry configured
    /// online while the run is offline would reach the network behind
    /// `--offline`, and there would be nothing in either type to stop it.
    pub(crate) fn serve(&mut self, access: Access, store_root: &Path) {
        for registry in self.by_name.values_mut() {
            registry.serve(access, store_root);
        }
    }

    pub fn from_config(config: &LocalConfig, root: &Path) -> Result<Self, String> {
        let mut by_name = BTreeMap::new();
        for (name, entry) in &config.registry {
            let index = entry.index.as_deref().ok_or_else(|| {
                format!("`[registry.{name}]` names no `index` to read packages from")
            })?;
            by_name.insert(
                name.clone(),
                Registry::trusting(name, index, root, &entry.trusted_keys)?,
            );
        }
        Ok(Self { by_name })
    }

    /// The registry a manifest named, which somebody had to configure.
    pub fn named(&self, name: &str) -> Result<&Registry, String> {
        self.by_name.get(name).ok_or_else(|| {
            let configured = self
                .by_name
                .keys()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>();
            let known = if configured.is_empty() {
                "no registry is configured in `.slopium/config.toml`".to_owned()
            } else {
                format!("configured: {}", configured.join(", "))
            };
            format!(
                "SL1030: registry `{name}` is not configured; add `[registry.{name}] index = \"...\"` to `.slopium/config.toml`. This toolchain ships no registry URL, so every one of them is a choice somebody made ({known})"
            )
        })
    }

    /// The registry a lockfile's `registry+<url>` refers to.
    pub fn at_index(&self, index: &str) -> Result<&Registry, String> {
        let index = normalize_index(index);
        self.by_name
            .values()
            .find(|registry| registry.index == index)
            .ok_or_else(|| {
                format!(
                    "SL1030: `{}` pins a package to the registry `{index}`, which is not configured here; add it to `.slopium/config.toml` or resolve again",
                    crate::lock::LOCK_FILE
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> IndexEntry {
        IndexEntry {
            name: "geometry".to_owned(),
            version: Version::new(1, 4, 0),
            dependencies: vec![
                IndexDependency {
                    name: "std".to_owned(),
                    requirement: VersionReq::parse("^0.4").unwrap(),
                    source: IndexSource::Toolchain,
                },
                IndexDependency {
                    name: "units".to_owned(),
                    requirement: VersionReq::parse("^2").unwrap(),
                    source: IndexSource::SameIndex,
                },
            ],
            checksum: crate::sha256::sha256(b"geometry"),
            yanked: false,
            signature: None,
        }
    }

    #[test]
    fn an_index_entry_round_trips() {
        let line = entry().render().unwrap();
        assert_eq!(IndexEntry::parse(&line, "index").unwrap(), entry());
    }

    /// `D-054`: a dependency that names no registry means the one the entry
    /// came from, and that is what an absent field has to decode to.
    #[test]
    fn a_dependency_naming_no_registry_stays_in_this_one() {
        let line = r#"{"name":"geometry","version":"1.0.0","dependencies":[{"name":"units","requirement":"^2"}],"checksum":"0000000000000000000000000000000000000000000000000000000000000000"}"#;
        let parsed = IndexEntry::parse(line, "index").unwrap();
        assert_eq!(parsed.dependencies[0].source, IndexSource::SameIndex);
        assert!(!parsed.yanked);
    }

    /// An index may grow a field without this client refusing to read it. That
    /// tolerance is what let `signature` arrive in v0.4.5 without every v0.4.4
    /// checkout breaking, and it is worth keeping for whatever is next.
    #[test]
    fn an_unknown_field_does_not_stop_the_index_being_read() {
        let line = r#"{"name":"geometry","version":"1.0.0","checksum":"0000000000000000000000000000000000000000000000000000000000000000","published":"2026-07-31"}"#;
        assert_eq!(
            IndexEntry::parse(line, "index").unwrap().version,
            Version::new(1, 0, 0)
        );
    }

    /// A signature survives the round trip, and an unsigned entry does not grow
    /// a null field that an older reader would have to understand.
    #[test]
    fn a_signed_entry_round_trips_and_an_unsigned_one_says_nothing() {
        let key = crate::signature::PrivateKey::generate().unwrap();
        let mut signed = entry();
        signed.signature = Some(key.sign(&signed.name, &signed.version, &signed.checksum));
        let line = signed.render().unwrap();
        assert_eq!(IndexEntry::parse(&line, "index").unwrap(), signed);
        assert!(!entry().render().unwrap().contains("signature"));
    }

    #[test]
    fn a_malformed_line_names_the_file_it_is_in() {
        let error = IndexEntry::parse("{", "index/ge/om/geometry.json").unwrap_err();
        assert!(error.contains("SL1036"), "{error}");
        assert!(error.contains("index/ge/om/geometry.json"), "{error}");
    }

    #[test]
    fn the_index_fans_out_by_name_length() {
        assert_eq!(index_path("a").unwrap(), "1/a.json");
        assert_eq!(index_path("ab").unwrap(), "2/ab.json");
        assert_eq!(index_path("abc").unwrap(), "3/a/abc.json");
        assert_eq!(index_path("geometry").unwrap(), "ge/om/geometry.json");
        assert!(index_path("../etc").is_err());
    }

    #[test]
    fn a_plaintext_index_is_refused_unless_it_is_loopback() {
        let root = Path::new("/tmp");
        assert!(Registry::new("public", "http://example.invalid/index", root).is_err());
        assert!(Registry::new("local", "http://127.0.0.1:8080/index", root).is_ok());
        assert!(Registry::new("local", "http://localhost:8080/index", root).is_ok());
        assert!(Registry::new("secure", "https://example.invalid/index", root).is_ok());
        assert!(Registry::new("odd", "ftp://example.invalid/index", root).is_err());
    }

    /// A configured relative path is a directory, because a local registry is a
    /// directory and dressing it up as a URL adds nothing.
    #[test]
    fn a_registry_may_be_a_directory() {
        let registry = Registry::new("test", "tests/registry", Path::new("/work")).unwrap();
        assert!(matches!(
            registry.transport,
            Transport::Directory(ref path) if path == Path::new("/work/tests/registry")
        ));
        let absolute = Registry::new("test", "file:///srv/registry/", Path::new("/work")).unwrap();
        assert_eq!(absolute.index(), "file:///srv/registry");
        assert!(matches!(
            absolute.transport,
            Transport::Directory(ref path) if path == Path::new("/srv/registry")
        ));
    }

    /// `D-053`: the message has to say that nothing is missing from the
    /// toolchain — the registry was never going to be there.
    #[test]
    fn an_unconfigured_registry_says_it_is_a_choice() {
        let error = Registries::default().named("default").unwrap_err();
        assert!(error.contains("SL1030"), "{error}");
        assert!(error.contains("ships no registry URL"), "{error}");
    }
}
