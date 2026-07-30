//! The library bundled with the compiler, written down once.
//!
//! These two module sources used to exist in three copies — in the compiler
//! library, in the project manager's cache keying, and in the language server —
//! and the language-item defaults in four. Three copies of a source string is
//! three chances for them to disagree about what `std` contains, and nothing
//! would have reported it (`D-034`).

use crate::archive::{prefix_for, write, Entry};
use crate::manifest::MANIFEST_FILE;
use crate::sha256::{sha256, Digest};
use crate::version::Version;

/// The package name the bundled library resolves under, and therefore the
/// namespace its modules take.
pub const STD_PACKAGE: &str = "std";

/// Module name and source for each module of the bundled library.
pub const STD_MODULES: &[(&str, &str)] = &[
    (
        "option",
        "(export Option)\n(enum Option (T) None (Some ((value T))))\n",
    ),
    (
        "result",
        "(export Result (Result:Ok :as Ok) (Result:Err :as Err))\n\
         (enum Result (T E)\n\
           (Ok ((value T)))\n\
           (Err ((error E))))\n",
    ),
];

/// The language items the bundled library supplies, already namespaced.
pub fn std_language_items() -> Vec<(String, String)> {
    STD_LANGUAGE_ITEMS
        .iter()
        .map(|(name, path)| ((*name).to_owned(), format!("{STD_PACKAGE}:{path}")))
        .collect()
}

const STD_LANGUAGE_ITEMS: &[(&str, &str)] = &[
    ("option", "option:Option"),
    ("result", "result:Result"),
    ("result-ok", "result:Ok"),
    ("result-err", "result:Err"),
];

/// The path reported for a bundled module in diagnostics. It is not a real
/// file, and the angle brackets keep it from looking like one.
pub fn std_module_path(module: &str) -> String {
    format!("<toolchain>/{STD_PACKAGE}/{module}.slp")
}

/// The bundled library written out as an ordinary package.
///
/// It has no `entry`, because it is a library and there is no module a build of
/// it would start from, and it declares the language items it supplies — which
/// is what makes it the standard library rather than what it is named
/// (`D-011`).
pub fn std_manifest(version: &Version) -> String {
    let mut manifest = format!(
        "# Written by slopium from the library bundled with the compiler.\n\
         # Editing it makes this copy stop matching its checksum.\n\n\
         [package]\n\
         name = \"{STD_PACKAGE}\"\n\
         version = \"{version}\"\n\
         source = \"src\"\n\n\
         [language-items]\n"
    );
    for (name, path) in STD_LANGUAGE_ITEMS {
        manifest.push_str(&format!("{name} = \"{path}\"\n"));
    }
    manifest
}

/// The bundled library as archive entries, under the usual package prefix.
pub fn std_entries(version: &Version) -> Vec<Entry> {
    let prefix = prefix_for(STD_PACKAGE, version);
    let mut entries = vec![Entry::file(
        format!("{prefix}/{MANIFEST_FILE}"),
        std_manifest(version).into_bytes(),
    )];
    for (module, source) in STD_MODULES {
        entries.push(Entry::file(
            format!("{prefix}/src/{module}.slp"),
            source.as_bytes().to_vec(),
        ));
    }
    entries
}

/// The bundled library's archive and the digest the lockfile records for it.
///
/// The library ships inside the compiler, so this is not something to fetch —
/// but it is bytes with a version, and hashing them is what lets a lock notice
/// a toolchain whose library changed without its version doing so.
pub fn std_archive(version: &Version) -> Result<(Vec<u8>, Digest), String> {
    let bytes = write(&std_entries(version))?;
    let digest = sha256(&bytes);
    Ok((bytes, digest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_items_name_modules_that_exist() {
        for (_, path) in std_language_items() {
            let module = path.split(':').nth(1).expect("namespaced language item");
            assert!(
                STD_MODULES.iter().any(|(name, _)| *name == module),
                "language item names missing module `{module}`"
            );
        }
    }

    /// The generated manifest has to be an ordinary one, because a vendored
    /// copy of the library is loaded by the same code as any other package.
    #[test]
    fn the_generated_manifest_describes_the_library() {
        let version = Version::new(1, 2, 3);
        let manifest: crate::manifest::Manifest =
            toml::from_str(&std_manifest(&version)).expect("the generated manifest parses");
        let package = manifest.package.expect("a package section");
        assert_eq!(package.name, STD_PACKAGE);
        assert!(package.entry.is_none(), "a library has no entry module");
        assert_eq!(
            manifest
                .language_items
                .entries()
                .into_iter()
                .map(|(name, path)| (name, format!("{STD_PACKAGE}:{path}")))
                .collect::<Vec<_>>(),
            std_language_items()
        );
    }

    /// Two runs of the same toolchain describe the same library, or the digest
    /// in every lockfile would be noise.
    #[test]
    fn the_archive_is_the_same_every_time() {
        let version = Version::new(1, 2, 3);
        assert_eq!(
            std_archive(&version).unwrap().0,
            std_archive(&version).unwrap().0
        );
    }

    #[test]
    fn every_module_exports_something() {
        for (name, source) in STD_MODULES {
            assert!(
                source.contains("(export"),
                "module `{name}` exports nothing"
            );
        }
    }
}
