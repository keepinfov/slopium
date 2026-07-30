//! The package archive: a tar with every source of variation removed (`D-039`).
//!
//! A published package has to hash to the same digest for everyone, so nothing
//! that differs between two machines may reach the bytes. Timestamps, owners,
//! permissions beyond the read/execute distinction, directory-read order and
//! the tar dialect are all pinned here: entries are sorted by path, `mtime` is
//! zero, uid and gid are zero, modes are `0644` for files and `0755` for
//! directories, and the format is plain ustar with no extension headers. There
//! is no compression in the format at all — the digest is over the tar, and a
//! transport may compress it if it likes.
//!
//! Everything lives under one prefix, `<name>-<version>/`, and nothing may
//! escape it. Symbolic links, hard links and device nodes are refused on the
//! way in and on the way out: a package is source text, and an archive that can
//! write outside the directory it is unpacked into is the oldest bug in the
//! format.

use crate::manifest::{Project, MANIFEST_FILE};
use crate::sha256::{sha256, Digest};
use crate::version::Version;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// The extension a package archive is written with.
pub const ARCHIVE_EXTENSION: &str = "sl.tar";

/// One tar block, and the unit every field of the format is measured in.
const BLOCK: usize = 512;

/// Trailing zero blocks are padded up to this, the historical blocking factor.
/// It costs a few kilobytes and keeps every tar implementation quiet.
const BLOCKING_FACTOR: usize = 20 * BLOCK;

/// What an entry is. A package holds source text and the directories that
/// organize it, and nothing else — there is deliberately no arm for a link.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryKind {
    Directory,
    File,
}

/// One archive member, named by a path relative to the archive itself.
///
/// The path always begins with the package prefix and never ends in a slash;
/// the trailing slash a directory header carries is added when it is written.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Entry {
    pub path: String,
    pub kind: EntryKind,
    pub bytes: Vec<u8>,
}

impl Entry {
    pub fn file(path: impl Into<String>, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            path: path.into(),
            kind: EntryKind::File,
            bytes: bytes.into(),
        }
    }

    pub fn directory(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            kind: EntryKind::Directory,
            bytes: Vec::new(),
        }
    }
}

/// The directory name every entry of a package archive sits under.
pub fn prefix_for(name: &str, version: &Version) -> String {
    format!("{name}-{version}")
}

/// Serialize entries into a package archive.
///
/// The result is canonical: the same tree gives the same bytes however the
/// entry list was built. Missing parent directories are filled in, entries are
/// sorted, and a duplicate path is an error rather than a last-writer-wins.
pub fn write(entries: &[Entry]) -> Result<Vec<u8>, String> {
    let mut canonical: BTreeMap<String, Entry> = BTreeMap::new();
    let mut prefix: Option<String> = None;
    for entry in entries {
        let path = check_path(&entry.path, entry.kind)?;
        let root = path
            .split('/')
            .next()
            .expect("a checked path has a first component")
            .to_owned();
        match &prefix {
            Some(existing) if *existing != root => {
                return Err(format!(
                    "SL1003: an archive holds one package, but `{existing}` and `{root}` are both at its top level"
                ))
            }
            Some(_) => {}
            None => prefix = Some(root),
        }
        // Every directory on the way down is an entry of its own, so unpacking
        // never has to invent one and two archives of the same tree cannot
        // differ by whether the walk happened to report a directory.
        let mut walked = String::new();
        let components: Vec<&str> = path.split('/').collect();
        for component in &components[..components.len() - 1] {
            if !walked.is_empty() {
                walked.push('/');
            }
            walked.push_str(component);
            canonical
                .entry(walked.clone())
                .or_insert_with(|| Entry::directory(walked.clone()));
        }
        if let Some(previous) = canonical.insert(path.to_owned(), entry.clone()) {
            if previous.kind == EntryKind::File || previous != *entry {
                return Err(format!("archive names `{path}` twice"));
            }
        }
    }
    if canonical.is_empty() {
        return Err("an archive cannot be empty".to_owned());
    }

    let mut output = Vec::new();
    for entry in canonical.values() {
        output.extend_from_slice(&header(entry)?);
        if entry.kind == EntryKind::File {
            output.extend_from_slice(&entry.bytes);
            let padding = (BLOCK - entry.bytes.len() % BLOCK) % BLOCK;
            output.resize(output.len() + padding, 0);
        }
    }
    output.resize(output.len() + 2 * BLOCK, 0);
    let tail = (BLOCKING_FACTOR - output.len() % BLOCKING_FACTOR) % BLOCKING_FACTOR;
    output.resize(output.len() + tail, 0);
    Ok(output)
}

/// Parse a package archive, refusing anything that could unpack outside itself.
///
/// This is the boundary that a downloaded archive crosses, so it is written to
/// distrust its input: an unknown entry type, a path with `..` in it, a second
/// top-level directory and a truncated block are each an error naming what was
/// found.
pub fn read(bytes: &[u8]) -> Result<Vec<Entry>, String> {
    if !bytes.len().is_multiple_of(BLOCK) || bytes.is_empty() {
        return Err(format!(
            "SL1004: an archive is a whole number of {BLOCK}-byte blocks, and this one is {} bytes",
            bytes.len()
        ));
    }
    let mut entries = Vec::new();
    let mut prefix: Option<String> = None;
    let mut offset = 0;
    while offset + BLOCK <= bytes.len() {
        let block = &bytes[offset..offset + BLOCK];
        offset += BLOCK;
        if block.iter().all(|byte| *byte == 0) {
            break;
        }
        if &block[257..262] != b"ustar" {
            return Err("SL1004: not a ustar archive".to_owned());
        }
        verify_checksum(block)?;

        let name = field(block, 0, 100)?;
        let stored_prefix = field(block, 345, 155)?;
        let joined = if stored_prefix.is_empty() {
            name
        } else {
            format!("{stored_prefix}/{name}")
        };
        let kind = match block[156] {
            b'0' | 0 => EntryKind::File,
            b'5' => EntryKind::Directory,
            other => {
                let described = match other {
                    b'1' => "a hard link",
                    b'2' => "a symbolic link",
                    b'3' | b'4' => "a device node",
                    b'6' => "a fifo",
                    _ => "an extension header",
                };
                return Err(format!(
                    "SL1002: `{joined}` is {described}; a package holds files and directories only"
                ));
            }
        };
        let path = check_path(joined.trim_end_matches('/'), kind)?;
        let root = path
            .split('/')
            .next()
            .expect("a checked path has a first component")
            .to_owned();
        match &prefix {
            Some(existing) if *existing != root => {
                return Err(format!(
                    "SL1003: an archive holds one package, but `{existing}` and `{root}` are both at its top level"
                ))
            }
            Some(_) => {}
            None => prefix = Some(root),
        }

        let size = octal(block, 124, 12)? as usize;
        if kind == EntryKind::Directory && size != 0 {
            return Err(format!("SL1004: directory `{path}` has a size"));
        }
        let end = offset
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| format!("SL1004: `{path}` runs past the end of the archive"))?;
        entries.push(Entry {
            path: path.to_owned(),
            kind,
            bytes: bytes[offset..end].to_vec(),
        });
        offset += size.div_ceil(BLOCK) * BLOCK;
    }
    if entries.is_empty() {
        return Err("SL1004: the archive holds no entries".to_owned());
    }
    Ok(entries)
}

/// The single top-level directory of an archive's entries.
pub fn prefix_of(entries: &[Entry]) -> Result<&str, String> {
    entries
        .first()
        .and_then(|entry| entry.path.split('/').next())
        .ok_or_else(|| "the archive holds no entries".to_owned())
}

/// Everything `slopium package` puts in an archive: the manifest and the source
/// tree, minus what a package has no business carrying.
pub fn package_entries(project: &Project) -> Result<Vec<Entry>, String> {
    let prefix = prefix_for(&project.name, &project.version);
    let filter = Filter::for_project(project)?;
    let mut entries = Vec::new();
    collect(&project.root, &prefix, "", &filter, &mut entries)?;
    if !entries
        .iter()
        .any(|entry| entry.path == format!("{prefix}/{MANIFEST_FILE}"))
    {
        return Err(format!(
            "`{MANIFEST_FILE}` is excluded from the package, but an archive without a manifest is not a package"
        ));
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

/// Archive a package as it stands on disk, and say what it hashes to.
pub fn package_archive(project: &Project) -> Result<(Vec<u8>, Digest), String> {
    let bytes = write(&package_entries(project)?)?;
    let digest = sha256(&bytes);
    Ok((bytes, digest))
}

/// Archive an existing directory under `prefix`, taking every file in it.
///
/// Used to re-derive the digest of a tree that is supposed to be a package's
/// contents — a vendored copy, or a checkout in the store — which is what makes
/// a checksum something that can be re-checked rather than merely recorded.
pub fn directory_archive(root: &Path, prefix: &str) -> Result<(Vec<u8>, Digest), String> {
    let mut entries = Vec::new();
    collect(root, prefix, "", &Filter::nothing(), &mut entries)?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let bytes = write(&entries)?;
    let digest = sha256(&bytes);
    Ok((bytes, digest))
}

/// What a package leaves out, or — when `include` is given — all it takes in.
struct Filter {
    include: Vec<String>,
    exclude: Vec<String>,
}

impl Filter {
    fn nothing() -> Self {
        Self {
            include: Vec::new(),
            exclude: Vec::new(),
        }
    }

    fn for_project(project: &Project) -> Result<Self, String> {
        let package = project
            .manifest
            .package
            .as_ref()
            .ok_or_else(|| "a virtual manifest defines no package to archive".to_owned())?;
        if !package.include.is_empty() && !package.exclude.is_empty() {
            return Err(
                "`include` and `exclude` cannot both be given; `include` already says what the package is"
                    .to_owned(),
            );
        }
        let mut exclude = package.exclude.clone();
        // Build output and version control are never part of a package, and
        // `.slopium` is this machine's configuration rather than the project's.
        exclude.push("target".to_owned());
        exclude.push(".git".to_owned());
        exclude.push(".slopium".to_owned());
        // `D-044`: a library is built as part of somebody else's graph, and its
        // own lock says nothing about how it will be resolved there.
        if project.is_library() {
            exclude.push(crate::lock::LOCK_FILE.to_owned());
        }
        // A vendored copy is this checkout's answer to where a dependency's
        // bytes live, and it travels with the configuration that points at it —
        // which is in `.slopium`, and is not going anywhere.
        for source in project.config.source.values() {
            if let Some(directory) = &source.directory {
                if let Some(directory) = directory.to_str() {
                    exclude.push(directory.to_owned());
                }
            }
        }
        Ok(Self {
            include: package.include.clone(),
            exclude,
        })
    }

    /// Whether a path relative to the package root belongs in the archive. The
    /// manifest is not up for discussion — an archive without one is not a
    /// package, so `include` does not have to remember to name it.
    fn admits(&self, relative: &str) -> bool {
        if relative == MANIFEST_FILE {
            return true;
        }
        if !self.include.is_empty() {
            return self.include.iter().any(|pattern| covers(pattern, relative));
        }
        !self.exclude.iter().any(|pattern| covers(pattern, relative))
    }

    /// Whether to walk into a directory at all. A directory is entered unless
    /// something excludes it outright; with `include` in force it is always
    /// entered, because a pattern may name a file deep inside it.
    fn enters(&self, relative: &str) -> bool {
        self.include.is_empty() || !self.exclude.iter().any(|pattern| covers(pattern, relative))
    }
}

fn collect(
    directory: &Path,
    prefix: &str,
    relative: &str,
    filter: &Filter,
    entries: &mut Vec<Entry>,
) -> Result<(), String> {
    let read = fs::read_dir(directory)
        .map_err(|error| format!("cannot read `{}`: {error}", directory.display()))?;
    for child in read {
        let child =
            child.map_err(|error| format!("cannot read `{}`: {error}", directory.display()))?;
        let name = child.file_name().to_string_lossy().into_owned();
        let path = child.path();
        let relative = if relative.is_empty() {
            name.clone()
        } else {
            format!("{relative}/{name}")
        };
        // `symlink_metadata`, not `metadata`: a symlink to a directory would
        // otherwise be walked as one and packaged as a copy of its target.
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot read `{}`: {error}", path.display()))?;
        if metadata.file_type().is_symlink() {
            if !filter.admits(&relative) {
                continue;
            }
            return Err(format!(
                "SL1002: `{relative}` is a symbolic link; a package holds files and directories only. Exclude it or replace it with the file itself"
            ));
        }
        if metadata.is_dir() {
            if !filter.enters(&relative) {
                continue;
            }
            let before = entries.len();
            collect(&path, prefix, &relative, filter, entries)?;
            // An empty directory carries nothing and would only be a way for
            // two trees with the same files to hash differently.
            if entries.len() > before {
                entries.push(Entry::directory(format!("{prefix}/{relative}")));
            }
            continue;
        }
        if !metadata.is_file() {
            return Err(format!(
                "SL1002: `{relative}` is neither a file nor a directory; a package holds source, not devices"
            ));
        }
        if !filter.admits(&relative) {
            continue;
        }
        let bytes = fs::read(&path)
            .map_err(|error| format!("cannot read `{}`: {error}", path.display()))?;
        entries.push(Entry::file(format!("{prefix}/{relative}"), bytes));
    }
    Ok(())
}

/// Whether a pattern names a path or a directory containing it.
///
/// `*` matches within one path component and `**` matches across them, so
/// `src/*.slp` is the modules of one directory and `**/*.slp` is all of them.
/// Naming a directory takes everything under it, which is what makes
/// `exclude = ["target"]` mean what a reader expects.
fn covers(pattern: &str, path: &str) -> bool {
    let mut prefix = path;
    loop {
        if glob(pattern, prefix) {
            return true;
        }
        match prefix.rfind('/') {
            Some(cut) => prefix = &prefix[..cut],
            None => return false,
        }
    }
}

fn glob(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    fn matches(pattern: &[char], text: &[char]) -> bool {
        match pattern.first() {
            None => text.is_empty(),
            Some('*') => {
                let (rest, crosses) = if pattern.get(1) == Some(&'*') {
                    (&pattern[2..], true)
                } else {
                    (&pattern[1..], false)
                };
                // A `**` followed by a slash also stands for no directory at
                // all, so `**/x` matches a bare `x`.
                if crosses && rest.first() == Some(&'/') && matches(&rest[1..], text) {
                    return true;
                }
                for taken in 0..=text.len() {
                    if !crosses && text[..taken].contains(&'/') {
                        break;
                    }
                    if matches(rest, &text[taken..]) {
                        return true;
                    }
                }
                false
            }
            Some(character) => {
                !text.is_empty() && text[0] == *character && matches(&pattern[1..], &text[1..])
            }
        }
    }
    matches(&pattern, &text)
}

/// Reject a path that could not be unpacked safely, wherever it came from.
fn check_path(path: &str, kind: EntryKind) -> Result<&str, String> {
    if path.is_empty() {
        return Err("SL1001: an archive entry has an empty path".to_owned());
    }
    if path.starts_with('/') {
        return Err(format!("SL1001: `{path}` is absolute"));
    }
    if path.contains('\\') || path.contains('\0') {
        return Err(format!("SL1001: `{path}` is not a portable path"));
    }
    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(format!(
                "SL1001: `{path}` escapes the package it is packed under"
            ));
        }
    }
    // The one entry allowed a single component is the prefix directory itself.
    if path.split('/').count() < 2 && kind == EntryKind::File {
        return Err(format!(
            "SL1001: `{path}` is a file at the archive's top level; everything sits under one `<name>-<version>/` directory"
        ));
    }
    Ok(path)
}

fn header(entry: &Entry) -> Result<[u8; BLOCK], String> {
    let mut block = [0u8; BLOCK];
    let name = match entry.kind {
        EntryKind::Directory => format!("{}/", entry.path),
        EntryKind::File => entry.path.clone(),
    };
    let (prefix, name) = split_name(&name)?;
    block[..name.len()].copy_from_slice(name.as_bytes());
    block[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());

    let (mode, kind, size) = match entry.kind {
        EntryKind::Directory => ("0000755", b'5', 0),
        EntryKind::File => ("0000644", b'0', entry.bytes.len()),
    };
    block[100..107].copy_from_slice(mode.as_bytes());
    block[108..115].copy_from_slice(b"0000000");
    block[116..123].copy_from_slice(b"0000000");
    let size = format!("{size:011o}");
    if size.len() != 11 {
        return Err(format!("`{}` is too large to archive", entry.path));
    }
    block[124..135].copy_from_slice(size.as_bytes());
    block[136..147].copy_from_slice(b"00000000000");
    block[156] = kind;
    block[257..263].copy_from_slice(b"ustar\0");
    block[263..265].copy_from_slice(b"00");

    // The checksum is computed with its own field read as spaces, and written
    // back as six octal digits, a NUL and a space — the one field of the format
    // that every implementation agrees on only by accident.
    block[148..156].copy_from_slice(b"        ");
    let sum: u32 = block.iter().map(|byte| u32::from(*byte)).sum();
    let rendered = format!("{sum:06o}\0 ");
    block[148..156].copy_from_slice(rendered.as_bytes());
    Ok(block)
}

/// Split a path into ustar's 155-byte prefix and 100-byte name.
fn split_name(name: &str) -> Result<(String, String), String> {
    if name.len() <= 100 {
        return Ok((String::new(), name.to_owned()));
    }
    // The longest prefix that fits, so the name field holds as little as it can
    // and the split is a function of the path alone.
    let cut = name[..156.min(name.len())]
        .rfind('/')
        .filter(|cut| name.len() - cut - 1 <= 100)
        .ok_or_else(|| format!("`{name}` is too long for a package archive"))?;
    Ok((name[..cut].to_owned(), name[cut + 1..].to_owned()))
}

fn verify_checksum(block: &[u8]) -> Result<(), String> {
    let stored = octal(block, 148, 8)?;
    let sum: u32 = block
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            if (148..156).contains(&index) {
                u32::from(b' ')
            } else {
                u32::from(*byte)
            }
        })
        .sum();
    if u64::from(sum) == stored {
        Ok(())
    } else {
        Err("SL1004: an archive header fails its own checksum".to_owned())
    }
}

fn field(block: &[u8], start: usize, length: usize) -> Result<String, String> {
    let bytes = &block[start..start + length];
    let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(length);
    std::str::from_utf8(&bytes[..end])
        .map(str::to_owned)
        .map_err(|_| "SL1004: an archive header is not text".to_owned())
}

fn octal(block: &[u8], start: usize, length: usize) -> Result<u64, String> {
    let text = field(block, start, length)?;
    let text = text.trim_matches(|character: char| character == ' ' || character == '\0');
    if text.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(text, 8)
        .map_err(|_| format!("SL1004: `{text}` is not an octal header field"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Entry> {
        vec![
            Entry::file(
                "demo-1.0.0/src/main.slp",
                b"(fn main () -> i32 0)\n".to_vec(),
            ),
            Entry::file("demo-1.0.0/Slopium.toml", b"[package]\n".to_vec()),
        ]
    }

    #[test]
    fn an_archive_is_blocks_and_round_trips() {
        let bytes = write(&sample()).unwrap();
        assert_eq!(bytes.len() % BLOCKING_FACTOR, 0);
        let entries = read(&bytes).unwrap();
        assert_eq!(
            entries.iter().map(|entry| &entry.path).collect::<Vec<_>>(),
            [
                "demo-1.0.0",
                "demo-1.0.0/Slopium.toml",
                "demo-1.0.0/src",
                "demo-1.0.0/src/main.slp",
            ]
        );
        assert_eq!(entries[3].bytes, b"(fn main () -> i32 0)\n");
    }

    #[test]
    fn entry_order_does_not_reach_the_bytes() {
        let mut reversed = sample();
        reversed.reverse();
        assert_eq!(write(&sample()).unwrap(), write(&reversed).unwrap());
    }

    #[test]
    fn a_missing_parent_directory_is_filled_in() {
        let entries =
            read(&write(&[Entry::file("demo-1.0.0/a/b/c.slp", b"x".to_vec())]).unwrap()).unwrap();
        assert_eq!(
            entries.iter().map(|entry| &entry.path).collect::<Vec<_>>(),
            [
                "demo-1.0.0",
                "demo-1.0.0/a",
                "demo-1.0.0/a/b",
                "demo-1.0.0/a/b/c.slp"
            ]
        );
    }

    #[test]
    fn a_path_that_escapes_is_refused_both_ways() {
        let error = write(&[Entry::file("demo-1.0.0/../escape", b"x".to_vec())]).unwrap_err();
        assert!(error.contains("SL1001"), "{error}");

        // Forge the header directly: the writer would never emit one.
        let mut bytes = write(&sample()).unwrap();
        let forged = b"demo-1.0.0/../escape";
        bytes[..forged.len()].copy_from_slice(forged);
        bytes[forged.len()..100].fill(0);
        let block: [u8; BLOCK] = bytes[..BLOCK].try_into().unwrap();
        let mut block = block;
        block[148..156].copy_from_slice(b"        ");
        let sum: u32 = block.iter().map(|byte| u32::from(*byte)).sum();
        block[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        bytes[..BLOCK].copy_from_slice(&block);
        let error = read(&bytes).unwrap_err();
        assert!(error.contains("SL1001"), "{error}");
    }

    #[test]
    fn a_symbolic_link_entry_is_refused() {
        let mut bytes = write(&sample()).unwrap();
        let mut block: [u8; BLOCK] = bytes[..BLOCK].try_into().unwrap();
        block[156] = b'2';
        block[148..156].copy_from_slice(b"        ");
        let sum: u32 = block.iter().map(|byte| u32::from(*byte)).sum();
        block[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        bytes[..BLOCK].copy_from_slice(&block);
        let error = read(&bytes).unwrap_err();
        assert!(error.contains("SL1002"), "{error}");
        assert!(error.contains("symbolic link"), "{error}");
    }

    #[test]
    fn two_packages_cannot_share_an_archive() {
        let error = write(&[
            Entry::file("demo-1.0.0/a.slp", b"x".to_vec()),
            Entry::file("other-1.0.0/a.slp", b"x".to_vec()),
        ])
        .unwrap_err();
        assert!(error.contains("SL1003"), "{error}");
    }

    #[test]
    fn a_tampered_header_fails_its_checksum() {
        let mut bytes = write(&sample()).unwrap();
        bytes[0] = b'x';
        let error = read(&bytes).unwrap_err();
        assert!(error.contains("SL1004"), "{error}");
    }

    #[test]
    fn a_long_path_uses_the_prefix_field() {
        let deep = format!("demo-1.0.0/{}/main.slp", ["directory"; 12].join("/"));
        assert!(deep.len() > 100);
        let entries = read(&write(&[Entry::file(deep.clone(), b"x".to_vec())]).unwrap()).unwrap();
        assert!(entries.iter().any(|entry| entry.path == deep));
    }

    #[test]
    fn patterns_name_files_and_the_directories_over_them() {
        assert!(covers("target", "target/dev/thing.o"));
        assert!(covers("src/*.slp", "src/main.slp"));
        assert!(!covers("src/*.slp", "src/deep/main.slp"));
        assert!(covers("**/*.slp", "src/deep/main.slp"));
        assert!(covers("**/*.slp", "main.slp"));
        assert!(!covers("src", "source/main.slp"));
    }
}
