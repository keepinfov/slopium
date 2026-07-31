//! Where resolution goes to turn a written source into bytes.
//!
//! Three things decide that, and they are held together here so that no command
//! has to reimplement the combination: the content-addressed store, the access
//! policy (`--offline`), and what the lockfile already pinned.
//!
//! The pins are what make a lock a lock. A dependency the lock names is not
//! re-resolved — a branch is not consulted, `git` is not run, and the commit
//! recorded last time is the commit built this time. That is true whether or not
//! `--locked` was given: `--locked` says *do not write a new lock*, while the
//! lock itself says *do not go looking again*. Resolution reaches for the
//! network only for a dependency nothing has pinned yet.

use crate::archive::{self, prefix_for};
use crate::git;
use crate::lock::Lockfile;
use crate::manifest::{Manifest, MANIFEST_FILE};
use crate::registry::{IndexEntry, Registries};
use crate::resolve::PackageId;
use crate::sha256::Digest;
use crate::source::{GitReference, SourceId};
use crate::std_library::std_archive;
use crate::store::{Access, Store};
use crate::version::Version;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// What resolution settled on for one git dependency.
#[derive(Clone, Debug)]
pub struct GitPin {
    pub source: SourceId,
    pub version: Version,
    pub checksum: Digest,
}

/// What the lockfile says about one package.
#[derive(Clone, Debug)]
pub struct Pin {
    pub source: SourceId,
    pub version: Version,
    pub checksum: Option<Digest>,
}

/// Which pins `slopium update` has been told to throw away.
#[derive(Clone, Debug, Default)]
enum Refresh {
    /// Every pin stands.
    #[default]
    Nothing,
    /// `slopium update`: resolve as though there were no lock.
    Everything,
    /// `slopium update -p name`: exactly these move, and nothing else may.
    These(BTreeSet<String>),
}

/// The store, the access policy, the configured registries, and the lock's
/// pins.
#[derive(Clone, Debug)]
pub struct Sources {
    store: Store,
    access: Access,
    /// Whether the caller has forbidden the lock from changing.
    locked: bool,
    /// What the lockfile pinned, by package name.
    pins: BTreeMap<String, Pin>,
    registries: Registries,
    refresh: Refresh,
    /// Versions `--precise` demands, by package name.
    precise: BTreeMap<String, Version>,
    /// Git pins already resolved this run. Backtracking can reach one
    /// dependency many times, and a fetch is not something to repeat.
    fetched: Arc<Mutex<BTreeMap<(String, GitReference), GitPin>>>,
}

impl Sources {
    pub fn new(store: Store, access: Access, locked: bool) -> Self {
        Self {
            store,
            access,
            locked,
            pins: BTreeMap::new(),
            registries: Registries::default(),
            refresh: Refresh::Nothing,
            precise: BTreeMap::new(),
            fetched: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn with_registries(mut self, registries: Registries) -> Self {
        self.registries = registries;
        self
    }

    /// Throw away every pin, which is what `slopium update` is.
    pub fn updating_everything(mut self) -> Self {
        self.refresh = Refresh::Everything;
        self
    }

    /// Throw away the pins of these packages and no others, which is what
    /// `slopium update -p` is — and what makes the lock diff prove it.
    pub fn updating(mut self, names: impl IntoIterator<Item = String>) -> Self {
        self.refresh = Refresh::These(names.into_iter().collect());
        self
    }

    /// Demand an exact version of a package, whatever the index offers.
    pub fn at_precisely(mut self, name: &str, version: Version) -> Self {
        self.precise.insert(name.to_owned(), version);
        self
    }

    pub fn registries(&self) -> &Registries {
        &self.registries
    }

    pub fn locked(&self) -> bool {
        self.locked
    }

    /// What the lock pinned this package to, unless it is being updated.
    pub fn pinned(&self, name: &str) -> Option<&Pin> {
        let stands = match &self.refresh {
            Refresh::Nothing => true,
            Refresh::Everything => false,
            Refresh::These(names) => !names.contains(name),
        };
        stands.then(|| self.pins.get(name)).flatten()
    }

    /// The version `--precise` demands for this package, if any.
    pub fn precise(&self, name: &str) -> Option<&Version> {
        self.precise.get(name)
    }

    /// Every name the lockfile knows, so `update -p` can say when it is aimed
    /// at a package that is not in the graph.
    pub fn pinned_names(&self) -> Vec<&str> {
        self.pins.keys().map(String::as_str).collect()
    }

    /// Take the lockfile's pins, ignoring entries it cannot make sense of.
    ///
    /// A malformed `source` here is not fatal: the lock is a build product, and
    /// an entry that cannot be read is an entry that pins nothing, which is
    /// exactly the state resolution already knows how to be in.
    pub fn with_lock(mut self, lock: Option<&Lockfile>) -> Self {
        let Some(lock) = lock else { return self };
        for package in &lock.packages {
            let Ok(source) = SourceId::from_lock_field(&package.source) else {
                continue;
            };
            self.pins.insert(
                package.name.clone(),
                Pin {
                    source,
                    version: package.version.clone(),
                    checksum: package.checksum,
                },
            );
        }
        self
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn access(&self) -> Access {
        self.access
    }

    /// Settle a git dependency on one commit and one archive digest.
    ///
    /// The lock's answer is preferred and costs nothing — no fetch, no store
    /// read, not even a directory listing — which is what lets a vendored,
    /// locked project build with no `git` on the machine at all.
    pub fn pin_git(
        &self,
        declared: &str,
        url: &str,
        reference: &GitReference,
    ) -> Result<GitPin, String> {
        if let Some(pin) = self.pinned_git(declared, url, reference) {
            return pin;
        }
        let key = (url.to_owned(), reference.clone());
        if let Some(fetched) = self.fetched.lock().unwrap().get(&key) {
            return Ok(fetched.clone());
        }
        if self.locked {
            return Err(format!(
                "`{declared}` is not pinned by `{}` and --locked was given; run without it to resolve {reference} of `{url}`",
                crate::lock::LOCK_FILE
            ));
        }
        if self.access == Access::Offline {
            return Err(format!(
                "SL1011: `{declared}` is not pinned by `{}`, and `--offline` forbids running git to find {reference} of `{url}`",
                crate::lock::LOCK_FILE
            ));
        }

        let rev = git::pin(self.store.root(), url, reference)?;
        let source = SourceId::Git {
            url: url.to_owned(),
            reference: reference.clone(),
            rev,
        };
        let (bytes, version) = self.archive_from_git(declared, &source)?;
        let checksum = self.store.insert(&bytes)?;
        let pin = GitPin {
            source,
            version,
            checksum,
        };
        self.fetched.lock().unwrap().insert(key, pin.clone());
        Ok(pin)
    }

    /// Every version of a package a registry publishes, newest first.
    ///
    /// Reading an index is reaching for the network, so it obeys the same two
    /// rules a git fetch does: `--locked` forbids resolving a name the lock
    /// does not already pin, and `--offline` forbids the reach itself. What
    /// makes a fully pinned project resolve without either is that the caller
    /// asks `pinned` first and never gets here.
    pub fn published(&self, declared: &str, index: &str) -> Result<Vec<IndexEntry>, String> {
        if self.locked {
            return Err(format!(
                "`{declared}` is not pinned by `{}` and --locked was given; run without it to select a version from `{index}`",
                crate::lock::LOCK_FILE
            ));
        }
        if self.access == Access::Offline {
            return Err(format!(
                "SL1011: `{declared}` is not pinned by `{}`, and `--offline` forbids reading the index of `{index}` to find a version",
                crate::lock::LOCK_FILE
            ));
        }
        let registry = self.registries.at_index(index)?;
        let published = registry.versions(declared)?;
        let mut entries = published.as_ref().clone();
        entries.sort_by(|left, right| right.version.cmp(&left.version));
        Ok(entries)
    }

    /// The lock's answer for this dependency, if it has one that still applies.
    fn pinned_git(
        &self,
        declared: &str,
        url: &str,
        reference: &GitReference,
    ) -> Option<Result<GitPin, String>> {
        let pin = self.pinned(declared)?;
        // The reference is part of the source id (`D-049`), so a manifest that
        // moved from one branch to another does not match its own lock entry
        // and is resolved again.
        let SourceId::Git {
            url: pinned_url,
            reference: pinned_reference,
            ..
        } = &pin.source
        else {
            return None;
        };
        if pinned_url != url || pinned_reference != reference {
            return None;
        }
        Some(match pin.checksum {
            Some(checksum) => Ok(GitPin {
                source: pin.source.clone(),
                version: pin.version.clone(),
                checksum,
            }),
            None => Err(format!(
                "`{}` records `{declared}` as a git package with no checksum; delete it and resolve again",
                crate::lock::LOCK_FILE
            )),
        })
    }

    /// The tree of a resolved package, put in the store first if it is not
    /// already there.
    ///
    /// Everything a build reads from the store comes through here, so the
    /// verification the store does before it unpacks anything is not something
    /// a caller can forget to ask for.
    pub fn checkout(&self, id: &PackageId, checksum: &Digest) -> Result<PathBuf, String> {
        let described = id.to_string();
        if !self.store.holds(checksum) {
            let bytes = match &id.source {
                SourceId::Toolchain => std_archive(&id.version)?.0,
                SourceId::Git { .. } => {
                    if self.access == Access::Offline {
                        return Err(format!(
                            "SL1011: `{described}` is not in the package store — it needs {checksum} — and `--offline` forbids fetching it"
                        ));
                    }
                    self.archive_from_git(&id.name, &id.source)?.0
                }
                SourceId::Registry { index } => {
                    if self.access == Access::Offline {
                        return Err(format!(
                            "SL1011: `{described}` is not in the package store — it needs {checksum} — and `--offline` forbids downloading it from `{index}`"
                        ));
                    }
                    self.registries
                        .at_index(index)?
                        .archive(&id.name, &id.version)?
                }
                SourceId::Path(path) => {
                    return Err(format!(
                        "`{described}` is the directory `{}`; there is nothing to fetch and nothing to pin",
                        path.display()
                    ))
                }
            };
            // The digest is checked before the archive is read, let alone
            // stored: bytes that are not the ones this graph resolved should
            // not get as far as being parsed, and what they would have parsed
            // to is not the interesting thing to report about them.
            let digest = crate::sha256::sha256(&bytes);
            if digest != *checksum {
                return Err(match &id.source {
                    SourceId::Registry { index } => format!(
                        "SL1034: `{index}` served a `{described}` that hashes to {digest}, but {checksum} is what was published for it. The bytes are not the ones this graph resolved"
                    ),
                    _ => format!(
                        "SL1022: `{described}` now archives to {digest}, but the lock records {checksum}. The source has changed underneath a pinned commit"
                    ),
                });
            }
            self.store.insert(&bytes)?;
        }
        self.store.checkout(checksum, &described, self.access)
    }

    /// A git commit's tree as a package archive, and the version it declares.
    ///
    /// The prefix an archive is written under is `<name>-<version>/`, and the
    /// version is inside the manifest being archived — so the tree is exported
    /// once, read for what it calls itself, and only then given a prefix and
    /// written out.
    fn archive_from_git(
        &self,
        declared: &str,
        source: &SourceId,
    ) -> Result<(Vec<u8>, Version), String> {
        let SourceId::Git { url, rev, .. } = source else {
            return Err(format!("`{declared}` is not a git package"));
        };
        let entries = git::export(self.store.root(), url, rev)?;
        let manifest = entries
            .iter()
            .find(|entry| entry.path == MANIFEST_FILE)
            .ok_or_else(|| {
                format!("`{url}` at {rev} has no `{MANIFEST_FILE}`, so it is not a package")
            })?;
        let text = std::str::from_utf8(&manifest.bytes)
            .map_err(|_| format!("`{MANIFEST_FILE}` in `{url}` at {rev} is not text"))?;
        let parsed: Manifest = toml::from_str(text)
            .map_err(|error| format!("cannot parse `{MANIFEST_FILE}` in `{url}`: {error}"))?;
        let package = parsed
            .package
            .as_ref()
            .ok_or_else(|| format!("`{url}` at {rev} defines a workspace and no package"))?;
        if package.name != declared {
            return Err(format!(
                "`{declared}` is taken from `{url}`, but the package there is named `{}`; the key in `[dependencies]` must be the package name",
                package.name
            ));
        }
        let version = package
            .version
            .resolve(
                "version",
                parsed
                    .workspace
                    .as_ref()
                    .and_then(|section| section.package.version.as_ref()),
            )
            .map_err(|error| format!("`{MANIFEST_FILE}` in `{url}`: {error}"))?
            .clone();
        let prefixed = archive::under_prefix(&entries, &prefix_for(declared, &version));
        Ok((archive::write(&prefixed)?, version))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lock::LockedPackage;

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    fn lock(source: &str, checksum: Option<Digest>) -> Lockfile {
        Lockfile {
            format: crate::lock::LOCK_FORMAT,
            packages: vec![LockedPackage {
                name: "geometry".to_owned(),
                version: Version::new(1, 4, 0),
                source: source.to_owned(),
                dependencies: Vec::new(),
                checksum,
            }],
        }
    }

    fn sources(lock: &Lockfile, access: Access, locked: bool) -> Sources {
        Sources::new(Store::at("/nonexistent"), access, locked).with_lock(Some(lock))
    }

    /// The whole point of a pin: an answer that costs no fetch and no store
    /// access, which is why the store above points nowhere.
    #[test]
    fn a_pinned_reference_is_not_resolved_again() {
        let digest = crate::sha256::sha256(b"geometry");
        let lock = lock(
            &format!("git+https://example.invalid/geometry.git?branch=main#{COMMIT}"),
            Some(digest),
        );
        let pin = sources(&lock, Access::Offline, false)
            .pin_git(
                "geometry",
                "https://example.invalid/geometry.git",
                &GitReference::Branch("main".to_owned()),
            )
            .unwrap();
        assert_eq!(pin.version, Version::new(1, 4, 0));
        assert_eq!(pin.checksum, digest);
        assert_eq!(
            pin.source,
            SourceId::Git {
                url: "https://example.invalid/geometry.git".to_owned(),
                reference: GitReference::Branch("main".to_owned()),
                rev: COMMIT.to_owned(),
            }
        );
    }

    /// `D-049`: the manifest moved to another branch, so the lock no longer
    /// describes what was asked for and must not be believed.
    #[test]
    fn a_changed_reference_does_not_match_its_pin() {
        let lock = lock(
            &format!("git+https://example.invalid/geometry.git?branch=main#{COMMIT}"),
            Some(crate::sha256::sha256(b"geometry")),
        );
        let error = sources(&lock, Access::Offline, false)
            .pin_git(
                "geometry",
                "https://example.invalid/geometry.git",
                &GitReference::Branch("next".to_owned()),
            )
            .unwrap_err();
        assert!(error.contains("SL1011"), "{error}");
        assert!(error.contains("branch `next`"), "{error}");
    }

    /// A URL that moved is a different source, pinned or not.
    #[test]
    fn a_changed_url_does_not_match_its_pin() {
        let lock = lock(
            &format!("git+https://example.invalid/geometry.git#{COMMIT}"),
            Some(crate::sha256::sha256(b"geometry")),
        );
        let error = sources(&lock, Access::Online, true)
            .pin_git(
                "geometry",
                "https://elsewhere.invalid/geometry.git",
                &GitReference::DefaultBranch,
            )
            .unwrap_err();
        assert!(error.contains("--locked"), "{error}");
    }

    #[test]
    fn an_unpinned_dependency_is_refused_under_locked() {
        let error = Sources::new(Store::at("/nonexistent"), Access::Online, true)
            .pin_git(
                "geometry",
                "https://example.invalid/geometry.git",
                &GitReference::DefaultBranch,
            )
            .unwrap_err();
        assert!(error.contains("not pinned"), "{error}");
        assert!(error.contains("--locked"), "{error}");
    }

    /// A git entry without a checksum pins a commit and not its bytes, which is
    /// half a pin — and half a pin is what verification exists to notice.
    #[test]
    fn a_git_pin_without_a_checksum_is_refused() {
        let lock = lock(
            &format!("git+https://example.invalid/geometry.git#{COMMIT}"),
            None,
        );
        let error = sources(&lock, Access::Offline, false)
            .pin_git(
                "geometry",
                "https://example.invalid/geometry.git",
                &GitReference::DefaultBranch,
            )
            .unwrap_err();
        assert!(error.contains("no checksum"), "{error}");
    }

    /// A path entry in the lock says nothing about a git dependency of the same
    /// name, so it is not a pin.
    #[test]
    fn a_pin_of_another_source_is_not_a_pin() {
        let lock = lock("path+../geometry", None);
        let error = sources(&lock, Access::Offline, false)
            .pin_git(
                "geometry",
                "https://example.invalid/geometry.git",
                &GitReference::DefaultBranch,
            )
            .unwrap_err();
        assert!(error.contains("SL1011"), "{error}");
    }
}
