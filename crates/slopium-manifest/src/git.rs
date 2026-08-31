//! Fetching a package out of a git repository, by asking `git` to do it.
//!
//! `D-037` allows calling out to programs that already know about transports
//! and credentials, which is why `cc` links and why `git` fetches: an
//! implementation of the wire protocols in here would be a second thing to keep
//! correct and a second thing to keep secure, and it would still not know about
//! the user's SSH agent or their credential helper.
//!
//! What is kept is a bare repository per URL under `$SLOPIUM_HOME/git/db`,
//! fetched with an explicit refspec so nothing depends on how a particular git
//! is configured. Nothing is ever checked out there. A commit becomes a package
//! through `git archive`, whose tar is read for its paths and contents and for
//! nothing else — the bytes that get stored are written back out through the
//! ordinary archive writer, so a git package is the same kind of object as a
//! published one (`D-050`).

use crate::archive::{self, Entry};
use crate::codes;
use crate::sha256::sha256;
use crate::source::{check_commit, GitReference};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// The bare repository a URL is fetched into.
///
/// Named by a readable stem and the digest of the URL, so two repositories with
/// the same last path component do not share a directory and the name still
/// says which repository it is when somebody looks in the store.
pub fn database(store_root: &Path, url: &str) -> PathBuf {
    let stem = url
        .trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("repository")
        .trim_end_matches(".git");
    let stem: String = stem
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        .collect();
    let stem = if stem.is_empty() {
        "repository".to_owned()
    } else {
        stem
    };
    let digest = sha256(url.as_bytes()).to_string();
    store_root
        .join("git/db")
        .join(format!("{stem}-{}", &digest[..16]))
}

/// Fetch `url` and resolve `reference` to the commit it names today.
///
/// Every call fetches, because that is what resolving a reference means: a
/// branch that has not moved costs one round trip, and a branch that has is the
/// only way to learn the new commit. Callers that already know the commit — a
/// lock that pins one — use [`export`] and never come here.
pub fn pin(store_root: &Path, url: &str, reference: &GitReference) -> Result<String, String> {
    let database = database(store_root, url);
    initialize(&database, url)?;

    // The default branch is whatever the remote says its `HEAD` is, and only
    // the remote can say. Asking before the fetch means the answer describes
    // the same conversation the fetch is about to have.
    let wanted = match reference {
        GitReference::DefaultBranch => default_branch(url)?,
        GitReference::Branch(branch) => format!("refs/heads/{branch}"),
        GitReference::Tag(tag) => format!("refs/tags/{tag}"),
        GitReference::Rev(rev) => rev.clone(),
    };

    // A revision that is already here needs nothing from the network: it names
    // one immutable object, and the object cannot have changed.
    if matches!(reference, GitReference::Rev(_)) {
        if let Ok(rev) = rev_parse(&database, &wanted) {
            return Ok(rev);
        }
    }
    fetch(&database, url)?;
    rev_parse(&database, &wanted)
        .map_err(|_| format!("{}: `{url}` has no {reference}", codes::GIT_REFERENCE))
}

/// Whether a commit is already in the local database, so no fetch is needed.
pub fn holds(store_root: &Path, url: &str, rev: &str) -> bool {
    let database = database(store_root, url);
    database.is_dir() && rev_parse(&database, rev).is_ok()
}

/// A commit's tree, as the entries of a package archive.
///
/// Fetches first if the commit is not already here, so a lock that pins a
/// commit no local database has yet still builds. The returned paths are
/// relative to the repository root; giving them a package prefix is the
/// caller's job, because only the caller knows the version.
pub fn export(store_root: &Path, url: &str, rev: &str) -> Result<Vec<Entry>, String> {
    check_commit(rev)?;
    let database = database(store_root, url);
    if !holds(store_root, url, rev) {
        initialize(&database, url)?;
        fetch(&database, url)?;
        rev_parse(&database, rev).map_err(|_| {
            format!(
                "{}: `{url}` has no commit {rev}; it may have been rewritten",
                codes::GIT_REFERENCE
            )
        })?;
    }
    let output = run(
        Command::new("git")
            .arg("--git-dir")
            .arg(&database)
            .args(["archive", "--format=tar", rev]),
        "export a commit",
    )?;
    let entries = archive::read_exported(&output.stdout)?;
    if entries.iter().any(|entry| entry.path == ".gitmodules") {
        return Err(format!(
            "{}: the package at `{url}` uses git submodules, which this toolchain does not fetch; its build would be missing whatever they hold",
            codes::GIT_SUBMODULES
        ));
    }
    Ok(entries)
}

/// Create the bare database if it is not there, and point it at `url`.
///
/// `git init` rather than `git clone`, so the refspec below is the only thing
/// that decides what the database holds, and re-pointing an existing database
/// at a moved URL is a configuration change rather than a re-clone.
fn initialize(database: &Path, url: &str) -> Result<(), String> {
    if !database.join("HEAD").is_file() {
        let parent = database.parent().expect("a database path has a parent");
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create `{}`: {error}", parent.display()))?;
        run(
            Command::new("git")
                .args(["init", "--bare", "--quiet"])
                .arg(database),
            "create a repository",
        )?;
    }
    // A database that already exists has an `origin`; one just created has not,
    // and a URL that moved is a change to the remote rather than a re-clone.
    let pointed = run(
        Command::new("git")
            .arg("--git-dir")
            .arg(database)
            .args(["remote", "set-url", "origin", url]),
        "configure a repository",
    )
    .is_ok();
    if !pointed {
        run(
            Command::new("git")
                .arg("--git-dir")
                .arg(database)
                .args(["remote", "add", "origin", url]),
            "configure a repository",
        )?;
    }
    Ok(())
}

fn fetch(database: &Path, url: &str) -> Result<(), String> {
    run(
        Command::new("git").arg("--git-dir").arg(database).args([
            "fetch",
            "--quiet",
            "--force",
            "--prune",
            "--no-tags",
            "origin",
            "+refs/heads/*:refs/heads/*",
            "+refs/tags/*:refs/tags/*",
        ]),
        &format!("fetch `{url}`"),
    )?;
    Ok(())
}

/// Which branch the remote's `HEAD` points at.
fn default_branch(url: &str) -> Result<String, String> {
    let output = run(
        Command::new("git").args(["ls-remote", "--symref", url, "HEAD"]),
        &format!("read the default branch of `{url}`"),
    )?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find_map(|line| {
            line.strip_prefix("ref: ")?
                .split_whitespace()
                .next()
                .map(str::to_owned)
        })
        .ok_or_else(|| {
            format!(
                "{}: `{url}` does not say which branch is its default; name one with `branch`",
                codes::GIT_REFERENCE
            )
        })
}

/// The commit a reference names, as forty hex digits.
fn rev_parse(database: &Path, reference: &str) -> Result<String, String> {
    // `^{commit}` makes an annotated tag resolve to what it tags rather than to
    // the tag object, which is not a thing that can be archived.
    let output = run(
        Command::new("git").arg("--git-dir").arg(database).args([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{reference}^{{commit}}"),
        ]),
        "resolve a revision",
    )?;
    let rev = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    check_commit(&rev)?;
    Ok(rev)
}

/// Run a git command, with everything about this machine's git configuration
/// held away from it.
///
/// A fetch that a user's `~/.gitconfig` can redirect — `url.*.insteadOf` is
/// exactly that — is a fetch whose lock entry does not say where the bytes came
/// from. So the environment is fixed here rather than trusted.
fn run(command: &mut Command, action: &str) -> Result<Output, String> {
    let output = command
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                format!(
                    "{}: `git` is not on PATH, and a git dependency is fetched by running it",
                    codes::GIT_COMMAND
                )
            } else {
                format!("{}: cannot {action}: {error}", codes::GIT_COMMAND)
            }
        })?;
    if output.status.success() {
        return Ok(output);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    Err(if stderr.is_empty() {
        format!("{}: cannot {action}", codes::GIT_COMMAND)
    } else {
        format!("{}: cannot {action}: {stderr}", codes::GIT_COMMAND)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::archive::EntryKind;
    use std::fs;

    /// A repository built here, in a temporary directory, because the test
    /// suite runs in a sandbox with no network and must stay that way.
    struct Repository {
        root: PathBuf,
        store: PathBuf,
    }

    impl Repository {
        fn new(label: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "slopium-git-{label}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = fs::remove_dir_all(&base);
            let root = base.join("origin");
            fs::create_dir_all(&root).unwrap();
            let repository = Self {
                root,
                store: base.join("home"),
            };
            repository.git(&["init", "--quiet", "--initial-branch=main"]);
            repository.git(&["config", "user.name", "Test"]);
            repository.git(&["config", "user.email", "test@example.invalid"]);
            repository
        }

        fn git(&self, args: &[&str]) -> String {
            let output = Command::new("git")
                .current_dir(&self.root)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .env("GIT_AUTHOR_DATE", "2026-07-31T00:00:00Z")
                .env("GIT_COMMITTER_DATE", "2026-07-31T00:00:00Z")
                .args(args)
                .output()
                .unwrap_or_else(|error| panic!("cannot run git {args:?}: {error}"));
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        }

        fn write(&self, relative: &str, contents: &str) {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, contents).unwrap();
        }

        fn commit(&self, message: &str) -> String {
            self.git(&["add", "--all"]);
            self.git(&["commit", "--quiet", "--message", message]);
            self.git(&["rev-parse", "HEAD"])
        }

        fn url(&self) -> String {
            self.root.display().to_string()
        }
    }

    impl Drop for Repository {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(self.root.parent().unwrap());
        }
    }

    fn package(repository: &Repository, version: &str) {
        repository.write(
            "Slopium.toml",
            &format!("[package]\nname = \"geometry\"\nversion = \"{version}\"\nsource = \"src\"\n"),
        );
        repository.write("src/lib.slp", "(fn area () -> i32 4)\n");
    }

    #[test]
    fn a_reference_resolves_to_the_commit_it_names() {
        let repository = Repository::new("references");
        package(&repository, "1.0.0");
        let first = repository.commit("first");
        repository.git(&["tag", "v1.0.0"]);
        repository.git(&["checkout", "--quiet", "-b", "next"]);
        package(&repository, "1.1.0");
        let second = repository.commit("second");
        repository.git(&["checkout", "--quiet", "main"]);

        let url = repository.url();
        let store = &repository.store;
        assert_eq!(
            pin(store, &url, &GitReference::DefaultBranch).unwrap(),
            first
        );
        assert_eq!(
            pin(store, &url, &GitReference::Branch("main".to_owned())).unwrap(),
            first
        );
        assert_eq!(
            pin(store, &url, &GitReference::Tag("v1.0.0".to_owned())).unwrap(),
            first
        );
        assert_eq!(
            pin(store, &url, &GitReference::Branch("next".to_owned())).unwrap(),
            second
        );
        // A short revision is a way of naming a commit, and resolves to all of
        // it — which is what the lock records.
        assert_eq!(
            pin(store, &url, &GitReference::Rev(second[..8].to_owned())).unwrap(),
            second
        );
    }

    #[test]
    fn a_reference_that_names_nothing_says_so() {
        let repository = Repository::new("absent");
        package(&repository, "1.0.0");
        repository.commit("first");
        let error = pin(
            &repository.store,
            &repository.url(),
            &GitReference::Branch("absent".to_owned()),
        )
        .unwrap_err();
        assert!(error.contains("SL1023"), "{error}");
        assert!(error.contains("branch `absent`"), "{error}");
    }

    /// The point of `D-050`: what a commit archives to is a function of the
    /// tree, and the same commit gives the same package on any machine and at
    /// any time.
    #[test]
    fn a_commit_archives_to_the_same_package_every_time() {
        let repository = Repository::new("export");
        package(&repository, "1.0.0");
        let commit = repository.commit("first");

        let entries = export(&repository.store, &repository.url(), &commit).unwrap();
        assert!(entries
            .iter()
            .any(|entry| entry.path == "Slopium.toml" && entry.kind == EntryKind::File));
        assert!(entries.iter().any(|entry| entry.path == "src/lib.slp"));

        let prefixed = archive::under_prefix(&entries, "geometry-1.0.0");
        let bytes = archive::write(&prefixed).unwrap();
        let again = archive::write(&archive::under_prefix(
            &export(&repository.store, &repository.url(), &commit).unwrap(),
            "geometry-1.0.0",
        ))
        .unwrap();
        assert_eq!(bytes, again);
        // And it is an ordinary package archive, readable by the ordinary
        // reader — which is what makes a git package the same kind of object as
        // a published one.
        assert!(archive::read(&bytes)
            .unwrap()
            .iter()
            .any(|entry| entry.path == "geometry-1.0.0/src/lib.slp"));
    }

    #[test]
    fn a_package_with_submodules_is_refused() {
        let repository = Repository::new("submodules");
        package(&repository, "1.0.0");
        repository.write(
            ".gitmodules",
            "[submodule \"vendor\"]\n\tpath = vendor\n\turl = https://example.invalid/x.git\n",
        );
        let commit = repository.commit("first");
        let error = export(&repository.store, &repository.url(), &commit).unwrap_err();
        assert!(error.contains("SL1021"), "{error}");
        assert!(error.contains("submodules"), "{error}");
    }

    /// A fetched commit stays fetched, so a build that already has one never
    /// needs the network again — which is what `--offline` relies on.
    #[test]
    fn a_fetched_commit_is_held_locally() {
        let repository = Repository::new("holds");
        package(&repository, "1.0.0");
        let commit = repository.commit("first");
        let url = repository.url();
        assert!(!holds(&repository.store, &url, &commit));
        pin(&repository.store, &url, &GitReference::DefaultBranch).unwrap();
        assert!(holds(&repository.store, &url, &commit));
    }

    #[test]
    fn two_repositories_with_one_name_do_not_share_a_database() {
        let root = Path::new("/store");
        assert_ne!(
            database(root, "https://one.invalid/geometry.git"),
            database(root, "https://two.invalid/geometry.git")
        );
        assert!(database(root, "https://one.invalid/geometry.git")
            .to_string_lossy()
            .contains("geometry-"));
    }
}
