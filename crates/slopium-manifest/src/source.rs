//! Where a package comes from.
//!
//! `SourceSpec` is what a manifest wrote; `SourceId` is what resolution settled
//! on and what the lockfile records. They are separate because a spec can be
//! under-determined (`git = "...", branch = "main"` names a branch, and a branch
//! moves) while an id must be exact enough to reproduce — `git+URL?branch=main#`
//! and forty hex digits. The `Registry` arm lands in v0.4.4; the shape is
//! settled now so the lockfile format does not have to change when it does.

use std::fmt;
use std::path::PathBuf;

/// Which commit of a repository a dependency asked for.
///
/// Kept in the source id as well as the spec (`D-049`): a manifest that changes
/// from one branch to another has to disagree with the lock, and the commit
/// alone cannot disagree with anything.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum GitReference {
    /// Whatever the repository says its `HEAD` is.
    DefaultBranch,
    Branch(String),
    Tag(String),
    /// A commit named directly — a full hash, a short one, or anything else
    /// `git rev-parse` understands.
    Rev(String),
}

impl GitReference {
    /// The lockfile query naming this reference, empty for the default branch.
    fn query(&self) -> String {
        match self {
            Self::DefaultBranch => String::new(),
            Self::Branch(name) => format!("?branch={name}"),
            Self::Tag(name) => format!("?tag={name}"),
            Self::Rev(rev) => format!("?rev={rev}"),
        }
    }

    fn parse(query: &str) -> Result<Self, String> {
        if let Some(name) = query.strip_prefix("branch=") {
            return Ok(Self::Branch(name.to_owned()));
        }
        if let Some(name) = query.strip_prefix("tag=") {
            return Ok(Self::Tag(name.to_owned()));
        }
        if let Some(rev) = query.strip_prefix("rev=") {
            return Ok(Self::Rev(rev.to_owned()));
        }
        Err(format!("unknown git reference `{query}`"))
    }
}

impl fmt::Display for GitReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DefaultBranch => formatter.write_str("the default branch"),
            Self::Branch(name) => write!(formatter, "branch `{name}`"),
            Self::Tag(name) => write!(formatter, "tag `{name}`"),
            Self::Rev(rev) => write!(formatter, "revision `{rev}`"),
        }
    }
}

/// A source as written in `[dependencies]`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceSpec {
    /// A directory, relative to the manifest that named it.
    Path(PathBuf),
    /// The library bundled with the compiler.
    Toolchain,
    /// A git repository, at whichever commit the reference names today.
    Git {
        url: String,
        reference: GitReference,
    },
}

/// A resolved source, exact enough to appear in `Slopium.lock`.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum SourceId {
    /// A canonical directory on this machine.
    Path(PathBuf),
    /// The library bundled with the compiler.
    Toolchain,
    /// One commit of a git repository, and how it was found.
    Git {
        url: String,
        reference: GitReference,
        /// A full forty-character commit hash, always.
        rev: String,
    },
}

impl SourceId {
    /// Whether this source is a working tree rather than an immutable artifact.
    ///
    /// A path dependency is edited in place, so the lockfile records no checksum
    /// for it — hashing one would rewrite the lock on every keystroke.
    pub fn is_mutable(&self) -> bool {
        matches!(self, Self::Path(_))
    }

    /// How `.slopium/config.toml` names this source in a `[source.<name>]`
    /// table. Every path package is its own directory and there is nothing to
    /// redirect, so only the sources with fetchable bytes have a name here.
    pub fn config_name(&self) -> &'static str {
        match self {
            Self::Path(_) => "path",
            Self::Toolchain => "toolchain",
            Self::Git { .. } => "git",
        }
    }

    /// The `source` field written to and read from the lockfile.
    pub fn to_lock_field(&self) -> String {
        match self {
            Self::Path(path) => format!("path+{}", path.display()),
            Self::Toolchain => "toolchain".to_owned(),
            Self::Git {
                url,
                reference,
                rev,
            } => format!("git+{url}{}#{rev}", reference.query()),
        }
    }

    pub fn from_lock_field(text: &str) -> Result<Self, String> {
        if text == "toolchain" {
            return Ok(Self::Toolchain);
        }
        if let Some(path) = text.strip_prefix("path+") {
            return Ok(Self::Path(PathBuf::from(path)));
        }
        if let Some(rest) = text.strip_prefix("git+") {
            // Split from the right: a repository URL may itself hold a `#` or a
            // `?`, and the parts this format adds are always the last of each.
            let (location, rev) = rest
                .rsplit_once('#')
                .ok_or_else(|| format!("git source `{text}` names no commit"))?;
            check_commit(rev)?;
            let (url, reference) = match location.rsplit_once('?') {
                Some((url, query)) => (url, GitReference::parse(query)?),
                None => (location, GitReference::DefaultBranch),
            };
            if url.is_empty() {
                return Err(format!("git source `{text}` names no repository"));
            }
            return Ok(Self::Git {
                url: url.to_owned(),
                reference,
                rev: rev.to_owned(),
            });
        }
        Err(format!("unknown package source `{text}`"))
    }
}

/// A commit is forty lowercase hex digits, and a lock that says otherwise is
/// pinning something this toolchain did not resolve.
pub fn check_commit(rev: &str) -> Result<(), String> {
    let well_formed = rev.len() == 40
        && rev
            .chars()
            .all(|character| matches!(character, '0'..='9' | 'a'..='f'));
    if well_formed {
        Ok(())
    } else {
        Err(format!(
            "`{rev}` is not a commit; a pinned git source records all forty hexadecimal digits"
        ))
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(path) => write!(formatter, "{}", path.display()),
            Self::Toolchain => formatter.write_str("toolchain"),
            Self::Git { url, rev, .. } => write!(formatter, "{url}#{rev}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

    fn git(reference: GitReference) -> SourceId {
        SourceId::Git {
            url: "https://example.invalid/geometry.git".to_owned(),
            reference,
            rev: COMMIT.to_owned(),
        }
    }

    #[test]
    fn lock_fields_round_trip() {
        for source in [
            SourceId::Toolchain,
            SourceId::Path(PathBuf::from("/tmp/geometry")),
            git(GitReference::DefaultBranch),
            git(GitReference::Branch("main".to_owned())),
            git(GitReference::Tag("v1.4.0".to_owned())),
            git(GitReference::Rev("0123456".to_owned())),
        ] {
            let field = source.to_lock_field();
            assert_eq!(SourceId::from_lock_field(&field).unwrap(), source);
        }
        assert!(SourceId::from_lock_field("registry+https://example").is_err());
    }

    /// The exact text is part of the lockfile's contract, since the whole value
    /// of the file is that it diffs cleanly.
    #[test]
    fn the_rendered_git_source_is_the_documented_one() {
        assert_eq!(
            git(GitReference::DefaultBranch).to_lock_field(),
            format!("git+https://example.invalid/geometry.git#{COMMIT}")
        );
        assert_eq!(
            git(GitReference::Branch("main".to_owned())).to_lock_field(),
            format!("git+https://example.invalid/geometry.git?branch=main#{COMMIT}")
        );
    }

    /// `D-049`: the reference is part of the identity, so a manifest that
    /// switches branches has something in the lock to disagree with.
    #[test]
    fn two_references_to_one_repository_are_two_sources() {
        assert_ne!(
            git(GitReference::Branch("main".to_owned())),
            git(GitReference::Branch("next".to_owned()))
        );
        assert_ne!(
            git(GitReference::DefaultBranch),
            git(GitReference::Tag("v1".to_owned()))
        );
    }

    #[test]
    fn a_git_source_without_a_full_commit_is_refused() {
        let error = SourceId::from_lock_field("git+https://example.invalid/x.git").unwrap_err();
        assert!(error.contains("names no commit"), "{error}");
        let error =
            SourceId::from_lock_field("git+https://example.invalid/x.git#abc123").unwrap_err();
        assert!(error.contains("forty"), "{error}");
        let error = SourceId::from_lock_field(&format!(
            "git+https://example.invalid/x.git#{}",
            "A".repeat(40)
        ))
        .unwrap_err();
        assert!(error.contains("forty"), "{error}");
    }

    #[test]
    fn only_a_working_tree_is_mutable() {
        assert!(SourceId::Path(PathBuf::from("/tmp/x")).is_mutable());
        assert!(!SourceId::Toolchain.is_mutable());
        assert!(!git(GitReference::DefaultBranch).is_mutable());
    }
}
