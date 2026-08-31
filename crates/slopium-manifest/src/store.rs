//! The content-addressed store: where a package's bytes live once they stop
//! changing.
//!
//! Two things are kept, and the difference between them is the whole design.
//! `archives/<digest>.sl.tar` is the package — the exact bytes somebody hashed,
//! signed and recorded in a lockfile. `store/<digest>/` is only its unpacked
//! form, a cache that exists because a compiler reads files and not tars. So the
//! archive is what gets verified, every time the tree is used, and the tree is
//! something that may be deleted and rebuilt at any moment without losing
//! anything.
//!
//! Verification happens *before* unpacking, never after: an archive that fails
//! its digest is never given the chance to write a single file. Unpacking goes
//! to a temporary directory and is renamed into place, so a build that arrives
//! halfway through another one's extraction sees either nothing or the finished
//! tree, and never a directory missing half its modules.

use crate::archive::{self, EntryKind};
use crate::codes;
use crate::sha256::{sha256, Digest};
use crate::signature::Signature;
use std::fs;
use std::path::{Path, PathBuf};

/// Whether a source is allowed to go looking for bytes it does not have.
///
/// Every source in this release is local, so nothing can currently fail on
/// `Offline` that would succeed on `Online` — the policy exists as one value
/// threaded to one place, so that git and registry sources have somewhere to
/// ask rather than a flag to reinvent.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Access {
    #[default]
    Online,
    Offline,
}

impl Access {
    pub fn new(offline: bool) -> Self {
        if offline {
            Self::Offline
        } else {
            Self::Online
        }
    }
}

#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// The store this machine uses: `$SLOPIUM_HOME`, or the XDG cache
    /// directory. It is a cache in the strict sense — deleting it costs a
    /// re-fetch and nothing else — which is why it defaults under
    /// `XDG_CACHE_HOME` rather than `XDG_DATA_HOME`.
    pub fn open() -> Result<Self, String> {
        if let Some(home) = std::env::var_os("SLOPIUM_HOME") {
            return Ok(Self::at(PathBuf::from(home)));
        }
        let cache = match std::env::var_os("XDG_CACHE_HOME") {
            Some(cache) if !cache.is_empty() => PathBuf::from(cache),
            _ => {
                let home = std::env::var_os("HOME").ok_or_else(|| {
                    "neither `SLOPIUM_HOME` nor `HOME` is set, so there is nowhere to keep the package store"
                        .to_owned()
                })?;
                PathBuf::from(home).join(".cache")
            }
        };
        Ok(Self::at(cache.join("slopium")))
    }

    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn archive_path(&self, digest: &Digest) -> PathBuf {
        self.root
            .join("archives")
            .join(format!("{digest}.{}", archive::ARCHIVE_EXTENSION))
    }

    pub fn checkout_path(&self, digest: &Digest) -> PathBuf {
        self.root.join("store").join(digest.to_string())
    }

    /// Where the signature of a stored archive lives, beside the archive.
    ///
    /// It follows the bytes into the store so that verification is a property
    /// of the build rather than of whichever project happened to download the
    /// package first (`D-058`).
    pub fn signature_path(&self, digest: &Digest) -> PathBuf {
        self.root.join("archives").join(format!(
            "{digest}.{}.{}",
            archive::ARCHIVE_EXTENSION,
            crate::signature::SIGNATURE_EXTENSION
        ))
    }

    /// The signature filed with an archive, if one was ever filed.
    pub fn signature(&self, digest: &Digest) -> Result<Option<Signature>, String> {
        let path = self.signature_path(digest);
        match fs::read_to_string(&path) {
            Ok(text) => Signature::parse(text.trim())
                .map(Some)
                .map_err(|error| format!("`{}`: {error}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("cannot read `{}`: {error}", path.display())),
        }
    }

    /// File a signature with an archive already in the store.
    pub fn insert_signature(&self, digest: &Digest, signature: &Signature) -> Result<(), String> {
        let path = self.signature_path(digest);
        let directory = path
            .parent()
            .expect("a signature path has a parent directory");
        fs::create_dir_all(directory)
            .map_err(|error| format!("cannot create `{}`: {error}", directory.display()))?;
        let temporary = directory.join(format!(".{digest}.{}", scratch_suffix()));
        fs::write(&temporary, format!("{signature}\n"))
            .map_err(|error| format!("cannot write `{}`: {error}", temporary.display()))?;
        fs::rename(&temporary, &path)
            .map_err(|error| format!("cannot write `{}`: {error}", path.display()))
    }

    pub fn holds(&self, digest: &Digest) -> bool {
        self.archive_path(digest).is_file()
    }

    /// Put an archive in the store and return what it is addressed by.
    ///
    /// An archive already present is left exactly as it is. The digest names
    /// the bytes, so a rewrite could only ever be a no-op — or a way to quietly
    /// repair a store somebody has edited, which is the one thing verification
    /// exists to notice.
    pub fn insert(&self, bytes: &[u8]) -> Result<Digest, String> {
        // Refuse to store what could not be unpacked later. A store full of
        // archives that fail at checkout time is worse than a failed insert.
        archive::read(bytes)?;
        let digest = sha256(bytes);
        let path = self.archive_path(&digest);
        if path.is_file() {
            return Ok(digest);
        }
        let directory = path
            .parent()
            .expect("an archive path has a parent directory");
        fs::create_dir_all(directory)
            .map_err(|error| format!("cannot create `{}`: {error}", directory.display()))?;
        let temporary = directory.join(format!(".{digest}.{}", scratch_suffix()));
        fs::write(&temporary, bytes)
            .map_err(|error| format!("cannot write `{}`: {error}", temporary.display()))?;
        fs::rename(&temporary, &path)
            .map_err(|error| format!("cannot write `{}`: {error}", path.display()))?;
        Ok(digest)
    }

    /// The unpacked form of a stored archive, verified and read-only.
    ///
    /// `described` is how the package is named in an error — the lockfile's
    /// `name vX.Y.Z` — because a digest alone tells nobody which dependency to
    /// go and look at.
    pub fn checkout(
        &self,
        digest: &Digest,
        described: &str,
        access: Access,
    ) -> Result<PathBuf, String> {
        let archive_path = self.archive_path(digest);
        let bytes = match fs::read(&archive_path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(match access {
                    Access::Offline => format!(
                        "{}: `{described}` is not in the package store — it needs {digest} — and `--offline` forbids fetching it",
                        codes::NOT_LOCAL
                    ),
                    Access::Online => format!(
                        "{}: `{described}` is not in the package store and needs {digest}, but no source in this toolchain can fetch it",
                        codes::NOT_LOCAL
                    ),
                })
            }
            Err(error) => {
                return Err(format!(
                    "cannot read `{}`: {error}",
                    archive_path.display()
                ))
            }
        };
        if sha256(&bytes) != *digest {
            return Err(format!(
                "{}: the stored archive for `{described}` is not the one it is filed under. Expected {digest}; delete `{}` and fetch it again",
                codes::STORE_MISMATCH,
                archive_path.display()
            ));
        }

        let destination = self.checkout_path(digest);
        if destination.is_dir() {
            return Ok(destination);
        }
        let entries = archive::read(&bytes)?;
        let parent = destination
            .parent()
            .expect("a checkout path has a parent directory");
        fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create `{}`: {error}", parent.display()))?;
        let temporary = parent.join(format!(".{digest}.{}", scratch_suffix()));
        if temporary.exists() {
            let _ = remove_tree(&temporary);
        }
        unpack(&entries, &temporary)?;
        seal(&temporary)?;
        if fs::rename(&temporary, &destination).is_err() {
            // Either somebody else finished first — in which case their tree is
            // this tree, since both came from these bytes — or the rename truly
            // failed and the directory is still missing.
            let _ = remove_tree(&temporary);
            if !destination.is_dir() {
                return Err(format!(
                    "cannot place the checkout of `{described}` at `{}`",
                    destination.display()
                ));
            }
        }
        Ok(destination)
    }
}

/// Write an archive's entries under `root`, which must not already exist.
///
/// The entries have been through `archive::read`, so none of them can escape;
/// this only has to create what they name, in the order they name it.
pub fn unpack(entries: &[archive::Entry], root: &Path) -> Result<(), String> {
    fs::create_dir_all(root)
        .map_err(|error| format!("cannot create `{}`: {error}", root.display()))?;
    for entry in entries {
        // Everything sits under one prefix directory, and the caller asked for
        // the contents rather than the wrapper.
        let relative = match entry.path.split_once('/') {
            Some((_, rest)) => rest,
            None => continue,
        };
        let path = root.join(relative);
        match entry.kind {
            EntryKind::Directory => fs::create_dir_all(&path)
                .map_err(|error| format!("cannot create `{}`: {error}", path.display()))?,
            EntryKind::File => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).map_err(|error| {
                        format!("cannot create `{}`: {error}", parent.display())
                    })?;
                }
                fs::write(&path, &entry.bytes)
                    .map_err(|error| format!("cannot write `{}`: {error}", path.display()))?;
            }
        }
    }
    Ok(())
}

/// Check that a materialized tree is still the package it claims to be.
///
/// Re-archiving the directory and comparing digests works because the archive
/// format has no room for anything but the file names and their contents
/// (`D-039`) — the tree that produced a digest is the only tree that reproduces
/// it. This is what a vendored copy is worth: bytes anyone can re-derive, rather
/// than a checksum taken on trust.
pub fn verify_tree(
    root: &Path,
    prefix: &str,
    expected: &Digest,
    described: &str,
) -> Result<(), String> {
    let (_, digest) = archive::directory_archive(root, prefix)?;
    if digest == *expected {
        return Ok(());
    }
    Err(format!(
        "{}: `{described}` at `{}` does not match its checksum. Expected {expected}, found {digest}; the copy has been edited since it was written",
        codes::VENDOR_MISMATCH,
        root.display()
    ))
}

/// Make every file in a tree read-only.
///
/// The directories stay writable on purpose: the point is that nobody edits a
/// stored package by accident, not that a stale checkout becomes impossible to
/// delete.
fn seal(root: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    for child in
        fs::read_dir(root).map_err(|error| format!("cannot read `{}`: {error}", root.display()))?
    {
        let child = child.map_err(|error| format!("cannot read `{}`: {error}", root.display()))?;
        let path = child.path();
        if path.is_dir() {
            seal(&path)?;
        } else {
            fs::set_permissions(&path, fs::Permissions::from_mode(0o444))
                .map_err(|error| format!("cannot seal `{}`: {error}", path.display()))?;
        }
    }
    Ok(())
}

/// Delete a tree whose files this process made read-only.
pub fn remove_tree(root: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    if !root.exists() {
        return Ok(());
    }
    for child in
        fs::read_dir(root).map_err(|error| format!("cannot read `{}`: {error}", root.display()))?
    {
        let child = child.map_err(|error| format!("cannot read `{}`: {error}", root.display()))?;
        let path = child.path();
        if path.is_dir() {
            remove_tree(&path)?;
        } else {
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o644));
            fs::remove_file(&path)
                .map_err(|error| format!("cannot remove `{}`: {error}", path.display()))?;
        }
    }
    fs::remove_dir(root).map_err(|error| format!("cannot remove `{}`: {error}", root.display()))
}

/// A suffix no other process is using, for a directory that is about to be
/// renamed into place.
pub(crate) fn scratch_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or_default();
    format!("{}-{nanos}.tmp", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::Entry;

    struct Scratch {
        root: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "slopium-store-{name}-{}-{}",
                std::process::id(),
                scratch_suffix()
            ));
            let _ = remove_tree(&root);
            fs::create_dir_all(&root).unwrap();
            Self { root }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = remove_tree(&self.root);
        }
    }

    fn sample() -> Vec<u8> {
        archive::write(&[
            Entry::file("demo-1.0.0/Slopium.toml", b"[package]\n".to_vec()),
            Entry::file(
                "demo-1.0.0/src/main.slp",
                b"(fn main () -> i32 0)\n".to_vec(),
            ),
        ])
        .unwrap()
    }

    #[test]
    fn a_checkout_is_the_archive_unpacked_and_read_only() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new("checkout");
        let store = Store::at(&scratch.root);
        let digest = store.insert(&sample()).unwrap();
        assert!(store.holds(&digest));

        let tree = store
            .checkout(&digest, "demo v1.0.0", Access::Online)
            .unwrap();
        assert_eq!(
            fs::read_to_string(tree.join("src/main.slp")).unwrap(),
            "(fn main () -> i32 0)\n"
        );
        let mode = fs::metadata(tree.join("src/main.slp"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o222, 0, "a stored file is writable");

        // Asking twice is the same answer, and does not unpack again.
        assert_eq!(
            store
                .checkout(&digest, "demo v1.0.0", Access::Online)
                .unwrap(),
            tree
        );
    }

    #[test]
    fn an_edited_stored_archive_fails_before_anything_is_unpacked() {
        let scratch = Scratch::new("edited");
        let store = Store::at(&scratch.root);
        let digest = store.insert(&sample()).unwrap();

        let mut bytes = fs::read(store.archive_path(&digest)).unwrap();
        let offset = bytes.len() - 3 * 512;
        bytes[offset] = bytes[offset].wrapping_add(1);
        fs::write(store.archive_path(&digest), &bytes).unwrap();

        let error = store
            .checkout(&digest, "demo v1.0.0", Access::Online)
            .unwrap_err();
        assert!(error.contains("SL1010"), "{error}");
        assert!(
            !store.checkout_path(&digest).exists(),
            "an archive that failed verification was unpacked anyway"
        );
    }

    #[test]
    fn an_absent_package_says_so_differently_when_offline() {
        let scratch = Scratch::new("absent");
        let store = Store::at(&scratch.root);
        let digest = sha256(&sample());
        let error = store
            .checkout(&digest, "demo v1.0.0", Access::Offline)
            .unwrap_err();
        assert!(error.contains("SL1011"), "{error}");
        assert!(error.contains("--offline"), "{error}");
        let error = store
            .checkout(&digest, "demo v1.0.0", Access::Online)
            .unwrap_err();
        assert!(error.contains("SL1011"), "{error}");
        assert!(!error.contains("--offline"), "{error}");
    }

    #[test]
    fn a_checked_out_tree_reproduces_its_own_digest() {
        let scratch = Scratch::new("verify");
        let store = Store::at(&scratch.root);
        let digest = store.insert(&sample()).unwrap();
        let tree = store
            .checkout(&digest, "demo v1.0.0", Access::Online)
            .unwrap();
        verify_tree(&tree, "demo-1.0.0", &digest, "demo v1.0.0").unwrap();

        let edited = scratch.root.join("edited");
        fs::create_dir_all(edited.join("src")).unwrap();
        fs::write(edited.join("Slopium.toml"), "[package]\n").unwrap();
        fs::write(edited.join("src/main.slp"), "(fn main () -> i32 1)\n").unwrap();
        let error = verify_tree(&edited, "demo-1.0.0", &digest, "demo v1.0.0").unwrap_err();
        assert!(error.contains("SL1012"), "{error}");
    }

    #[test]
    fn the_store_refuses_bytes_it_could_not_unpack_later() {
        let scratch = Scratch::new("garbage");
        let store = Store::at(&scratch.root);
        let error = store.insert(b"not a tar at all").unwrap_err();
        assert!(error.contains("SL1004"), "{error}");
    }
}
