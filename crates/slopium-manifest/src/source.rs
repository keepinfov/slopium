//! Where a package comes from.
//!
//! `SourceSpec` is what a manifest wrote; `SourceId` is what resolution settled
//! on and what the lockfile records. They are separate because a spec can be
//! relative and under-determined (`git = "...", branch = "main"`) while an id
//! must be exact enough to reproduce (`git+URL?rev=<40 hex>`). The `Git` and
//! `Registry` arms land in v0.4.3 and v0.4.4; the shape is settled now so the
//! lockfile format does not have to change when they do.

use std::fmt;
use std::path::PathBuf;

/// A source as written in `[dependencies]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceSpec {
    /// A directory, relative to the manifest that named it.
    Path(PathBuf),
    /// The library bundled with the compiler.
    Toolchain,
}

/// A resolved source, exact enough to appear in `Slopium.lock`.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SourceId {
    /// A canonical directory on this machine.
    Path(PathBuf),
    /// The library bundled with the compiler.
    Toolchain,
}

impl SourceId {
    /// Whether this source is a working tree rather than an immutable artifact.
    ///
    /// A path dependency is edited in place, so the lockfile records no checksum
    /// for it — hashing one would rewrite the lock on every keystroke.
    pub fn is_mutable(&self) -> bool {
        matches!(self, Self::Path(_))
    }

    /// The `source` field written to and read from the lockfile.
    pub fn to_lock_field(&self) -> String {
        match self {
            Self::Path(path) => format!("path+{}", path.display()),
            Self::Toolchain => "toolchain".to_owned(),
        }
    }

    pub fn from_lock_field(text: &str) -> Result<Self, String> {
        if text == "toolchain" {
            return Ok(Self::Toolchain);
        }
        if let Some(path) = text.strip_prefix("path+") {
            return Ok(Self::Path(PathBuf::from(path)));
        }
        Err(format!("unknown package source `{text}`"))
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(path) => write!(formatter, "{}", path.display()),
            Self::Toolchain => formatter.write_str("toolchain"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_fields_round_trip() {
        for source in [
            SourceId::Toolchain,
            SourceId::Path(PathBuf::from("/tmp/geometry")),
        ] {
            let field = source.to_lock_field();
            assert_eq!(SourceId::from_lock_field(&field).unwrap(), source);
        }
        assert!(SourceId::from_lock_field("registry+https://example").is_err());
    }

    #[test]
    fn only_a_working_tree_is_mutable() {
        assert!(SourceId::Path(PathBuf::from("/tmp/x")).is_mutable());
        assert!(!SourceId::Toolchain.is_mutable());
    }
}
