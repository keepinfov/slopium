//! The manager's diagnostic codes, as one table (`D-071`).
//!
//! This is the manager's half of the registry the compiler's
//! `codes::ALL` in `crates/slopic-core/src/diagnostic.rs` is the other
//! half of: every raise site takes its code from here, and the test below
//! holds the table and `docs/diagnostics.md` to the same set in both
//! directions. `SL1044` is raised by the Nix bridge in `flake.nix`, which
//! cannot read a Rust constant, and is listed for that reason — the table
//! is the registry of the family, not of this crate's share of it.

pub const ARCHIVE_PATH: &str = "SL1001";
pub const ARCHIVE_ENTRY: &str = "SL1002";
pub const ARCHIVE_TWO_PACKAGES: &str = "SL1003";
pub const ARCHIVE_MALFORMED: &str = "SL1004";
pub const STORE_MISMATCH: &str = "SL1010";
pub const NOT_LOCAL: &str = "SL1011";
pub const VENDOR_MISMATCH: &str = "SL1012";
pub const GIT_COMMAND: &str = "SL1020";
pub const GIT_SUBMODULES: &str = "SL1021";
pub const PIN_MOVED: &str = "SL1022";
pub const GIT_REFERENCE: &str = "SL1023";
pub const REGISTRY: &str = "SL1030";
pub const TWO_SOURCES: &str = "SL1031";
pub const UNPUBLISHABLE: &str = "SL1032";
pub const INDEX_DISAGREEMENT: &str = "SL1033";
pub const SERVED_MISMATCH: &str = "SL1034";
pub const ALL_YANKED: &str = "SL1035";
pub const INDEX_MALFORMED: &str = "SL1036";
pub const FETCH_FAILED: &str = "SL1037";
pub const UNSIGNED: &str = "SL1040";
pub const FORGED_SIGNATURE: &str = "SL1041";
pub const UNTRUSTED_KEY: &str = "SL1042";
pub const ALREADY_PUBLISHED: &str = "SL1043";
/// A locked source the Nix bridge cannot reproduce (`D-061`).
///
/// Thrown during evaluation in `flake.nix` rather than printed by `slopium`,
/// because that is where the refusal happens, so this constant names a code
/// no Rust raise site formats — the document and this table still have to
/// agree it exists.
pub const NIX_BRIDGE: &str = "SL1044";
pub const DEPENDENCY_SOURCE: &str = "SL1050";
pub const GIT_DEPENDENCY: &str = "SL1051";
pub const WORKSPACE_INHERITANCE: &str = "SL1052";
pub const MANIFEST_FIELD: &str = "SL1053";
pub const SOURCE_TABLE: &str = "SL1054";
pub const EDITION: &str = "SL1055";
pub const SELECTION: &str = "SL1060";
pub const NOT_A_MEMBER: &str = "SL1061";
pub const UNLISTED_MEMBER: &str = "SL1062";
pub const MEMBERS: &str = "SL1063";
pub const CYCLE: &str = "SL1070";
pub const WRONG_NAME: &str = "SL1071";
pub const TWO_VERSIONS: &str = "SL1072";
pub const NO_VERSION: &str = "SL1073";
pub const TWO_STDLIBS: &str = "SL1074";
pub const VENDOR_MISSING: &str = "SL1075";
pub const GIT_PATH_DEPENDENCY: &str = "SL1076";
pub const TOOLCHAIN_SOURCE: &str = "SL1077";
pub const NO_CHECKSUM: &str = "SL1078";
pub const LOCK_MALFORMED: &str = "SL1080";
pub const LOCK_FORMAT: &str = "SL1081";
pub const LOCKED: &str = "SL1082";
pub const PROTOCOL: &str = "SL1090";
pub const C_SOURCES: &str = "SL1100";
pub const LINKER_SCRIPT: &str = "SL1101";
pub const TARGET_MODULE: &str = "SL1102";
pub const UNFIXABLE: &str = "SL1110";
/// A manifest key this toolchain does not know, and ignores (`D-128`).
///
/// The one warning in the manager's range, the way `SL08xx` is the compiler's:
/// everything else this table holds is a refusal.
pub const UNKNOWN_KEY: &str = "SL1200";

pub const ALL: &[(&str, &str)] = &[
    (
        ARCHIVE_PATH,
        "archive entry names a path outside the package",
    ),
    (ARCHIVE_ENTRY, "archive entry is not a file or a directory"),
    (ARCHIVE_TWO_PACKAGES, "archive holds more than one package"),
    (ARCHIVE_MALFORMED, "archive is malformed"),
    (
        STORE_MISMATCH,
        "stored archive does not match the digest it is filed under",
    ),
    (
        NOT_LOCAL,
        "package or index is not held locally and cannot be fetched",
    ),
    (VENDOR_MISMATCH, "vendored copy does not match its checksum"),
    (GIT_COMMAND, "git command could not be run, or failed"),
    (GIT_SUBMODULES, "fetched package uses git submodules"),
    (
        PIN_MOVED,
        "pinned commit no longer archives to the digest the lock records",
    ),
    (
        GIT_REFERENCE,
        "git reference names no commit in the repository",
    ),
    (
        REGISTRY,
        "registry is not configured, or its index is not one this toolchain can reach",
    ),
    (
        TWO_SOURCES,
        "one package name is required from two different sources",
    ),
    (
        UNPUBLISHABLE,
        "published package depends on something it may not",
    ),
    (
        INDEX_DISAGREEMENT,
        "fetched manifest disagrees with the index entry that selected it",
    ),
    (
        SERVED_MISMATCH,
        "downloaded archive does not hash to what the index published",
    ),
    (
        ALL_YANKED,
        "every version that would satisfy a requirement is yanked",
    ),
    (INDEX_MALFORMED, "index file is malformed"),
    (FETCH_FAILED, "index or package could not be fetched"),
    (
        UNSIGNED,
        "registry requires a signature and the package carries none",
    ),
    (
        FORGED_SIGNATURE,
        "signature by a trusted key does not verify the package",
    ),
    (
        UNTRUSTED_KEY,
        "package is signed by a key that is not in trusted-keys",
    ),
    (ALREADY_PUBLISHED, "version is already in the index"),
    (NIX_BRIDGE, "Nix bridge cannot fetch a locked source"),
    (
        DEPENDENCY_SOURCE,
        "dependency entry names no source, or names several",
    ),
    (GIT_DEPENDENCY, "git dependency's reference is wrong"),
    (
        WORKSPACE_INHERITANCE,
        "workspace inheritance cannot be satisfied",
    ),
    (
        MANIFEST_FIELD,
        "manifest field is missing or has the wrong shape",
    ),
    (
        SOURCE_TABLE,
        "source table is incomplete or points at nothing",
    ),
    (
        EDITION,
        "manifest names an edition this toolchain does not have",
    ),
    (SELECTION, "selection is ambiguous or contradictory"),
    (
        NOT_A_MEMBER,
        "named package is not a member of this workspace",
    ),
    (
        UNLISTED_MEMBER,
        "package sits inside a workspace without being listed",
    ),
    (
        MEMBERS,
        "members is malformed, names a directory that is not there, or two members share a name",
    ),
    (CYCLE, "dependency cycle"),
    (
        WRONG_NAME,
        "dependency key is not the name of the package found",
    ),
    (TWO_VERSIONS, "one name is required at two versions"),
    (NO_VERSION, "no published version satisfies a requirement"),
    (TWO_STDLIBS, "two packages define language items"),
    (
        VENDOR_MISSING,
        "replaced or vendored package is missing, or is a different package",
    ),
    (
        GIT_PATH_DEPENDENCY,
        "git package declares a path dependency",
    ),
    (
        TOOLCHAIN_SOURCE,
        "toolchain source is named for something other than a bundled package",
    ),
    (NO_CHECKSUM, "lock entry needs a checksum and has none"),
    (LOCK_MALFORMED, "Slopium.lock is malformed"),
    (
        LOCK_FORMAT,
        "Slopium.lock is a format version this toolchain does not write",
    ),
    (
        LOCKED,
        "--locked was given and the lock would have to change",
    ),
    (
        PROTOCOL,
        "compiler and manager disagree about the protocol version",
    ),
    (
        C_SOURCES,
        "c-sources entry is absolute or leaves the package",
    ),
    (
        LINKER_SCRIPT,
        "linker-script is absolute or leaves the package",
    ),
    (
        TARGET_MODULE,
        "target modules entry names no file, leaves the package, or is absolute",
    ),
    (UNFIXABLE, "file slopium fix cannot mend whole"),
    (
        UNKNOWN_KEY,
        "manifest sets a key this toolchain does not know",
    ),
];

#[cfg(test)]
mod tests {
    use super::ALL;
    use std::collections::HashSet;

    #[test]
    fn manager_codes_are_documented_and_unique() {
        let mut raised = HashSet::new();
        for (code, description) in ALL {
            assert!(code.starts_with("SL1") && code.len() == 6);
            assert!(!description.is_empty());
            assert!(raised.insert(*code), "duplicate diagnostic code {code}");
        }

        // The codes are frozen, and `docs/diagnostics.md` is the registry: it
        // lists exactly the manager's codes, so a code raised and never
        // documented fails here the same way a code documented and never
        // raised does. The compiler's `SL0xxx` half is held to the same
        // document by the table in `crates/slopic-core/src/diagnostic.rs`,
        // which is why the comparison here takes only `SL1`.
        let contract = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/diagnostics.md");
        let contract =
            std::fs::read_to_string(contract).expect("docs/diagnostics.md is part of the clone");
        let mut documented = HashSet::new();
        let mut rest = contract.as_str();
        while let Some(found) = rest.find("SL") {
            let candidate = &rest[found..];
            let digits = candidate[2..]
                .bytes()
                .take_while(u8::is_ascii_digit)
                .count();
            if digits == 4 && candidate.starts_with("SL1") {
                documented.insert(&candidate[..6]);
            }
            rest = &candidate[2..];
        }
        for code in &raised {
            assert!(
                documented.contains(code),
                "{code} is raised by the manager and not documented in docs/diagnostics.md"
            );
        }
        for code in &documented {
            assert!(
                raised.contains(code),
                "{code} is documented in docs/diagnostics.md and nothing raises it"
            );
        }
    }
}
