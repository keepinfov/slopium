//! The bundled library as packages: their manifests, their archives, and the
//! digests a lockfile records for them.
//!
//! What the library *is* — its sources, its module names, its language items —
//! lives in `slopium-std`, which the compiler depends on as well. It used to
//! live here, and the compiler kept a second copy it had no way to check
//! against this one (`D-076`).
//!
//! There are two of them since `D-082`: `core`, and `std` which depends on it.
//! Each is an ordinary package with its own manifest and its own checksum, so
//! nothing here knows which is which.

use crate::archive::{prefix_for, write, Entry};
use crate::manifest::MANIFEST_FILE;
use crate::sha256::{sha256, Digest};
use crate::version::Version;

pub use slopium_std::{
    language_items_of, toolchain_module_path, toolchain_package, ToolchainPackage, CORE_PACKAGE,
    STD_PACKAGE, TOOLCHAIN_PACKAGES,
};

/// A bundled package written out as an ordinary package.
///
/// It has no `entry`, because it is a library and there is no module a build of
/// it would start from, and it declares the language items it supplies — which
/// is what makes it the standard library rather than what it is named
/// (`D-011`).
pub fn std_manifest(package: &ToolchainPackage, version: &Version) -> String {
    let mut manifest = format!(
        "# Written by slopium from the library bundled with the compiler.\n\
         # Editing it makes this copy stop matching its checksum.\n\n\
         [package]\n\
         name = \"{}\"\n\
         version = \"{version}\"\n\
         source = \"src\"\n",
        package.name
    );
    if !package.dependencies.is_empty() {
        manifest.push_str("\n[dependencies]\n");
        for dependency in package.dependencies {
            manifest.push_str(&format!("{dependency} = {{ toolchain = true }}\n"));
        }
    }
    if !package.language_items.is_empty() {
        manifest.push_str("\n[language-items]\n");
        for (name, path) in package.language_items {
            manifest.push_str(&format!("{name} = \"{path}\"\n"));
        }
    }
    manifest
}

/// A bundled package as archive entries, under the usual package prefix.
pub fn std_entries(package: &ToolchainPackage, version: &Version) -> Vec<Entry> {
    let prefix = prefix_for(package.name, version);
    let mut entries = vec![Entry::file(
        format!("{prefix}/{MANIFEST_FILE}"),
        std_manifest(package, version).into_bytes(),
    )];
    for (module, source) in package.modules {
        entries.push(Entry::file(
            format!("{prefix}/src/{module}.slp"),
            source.as_bytes().to_vec(),
        ));
    }
    entries
}

/// A bundled package's archive and the digest the lockfile records for it.
///
/// The library ships inside the compiler, so this is not something to fetch —
/// but it is bytes with a version, and hashing them is what lets a lock notice
/// a toolchain whose library changed without its version doing so.
pub fn std_archive(
    package: &ToolchainPackage,
    version: &Version,
) -> Result<(Vec<u8>, Digest), String> {
    let bytes = write(&std_entries(package, version))?;
    let digest = sha256(&bytes);
    Ok((bytes, digest))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The generated manifest has to be an ordinary one, because a vendored
    /// copy of the library is loaded by the same code as any other package.
    #[test]
    fn the_generated_manifest_describes_the_library() {
        let version = Version::new(1, 2, 3);
        for bundled in TOOLCHAIN_PACKAGES {
            let manifest: crate::manifest::Manifest =
                toml::from_str(&std_manifest(bundled, &version)).expect("the manifest parses");
            let package = manifest.package.expect("a package section");
            assert_eq!(package.name, bundled.name);
            assert!(package.entry.is_none(), "a library has no entry module");
            assert_eq!(
                manifest
                    .language_items
                    .entries()
                    .into_iter()
                    .map(|(name, path)| (name, format!("{}:{path}", bundled.name)))
                    .collect::<Vec<_>>(),
                language_items_of(bundled.name)
            );
        }
    }

    /// `std` depends on `core`, and it has to say so in the manifest a resolver
    /// reads — the dependency is not something the resolver can guess (`D-082`).
    #[test]
    fn a_declared_dependency_reaches_the_manifest() {
        let version = Version::new(1, 2, 3);
        let std = toolchain_package(STD_PACKAGE).expect("the std package");
        let manifest: crate::manifest::Manifest =
            toml::from_str(&std_manifest(std, &version)).expect("the manifest parses");
        let dependency = manifest
            .dependencies
            .get(CORE_PACKAGE)
            .expect("`std` depends on `core`");
        assert!(dependency.toolchain.is_some(), "from the toolchain");
    }

    /// Two runs of the same toolchain describe the same library, or the digest
    /// in every lockfile would be noise.
    #[test]
    fn the_archive_is_the_same_every_time() {
        let version = Version::new(1, 2, 3);
        for package in TOOLCHAIN_PACKAGES {
            assert_eq!(
                std_archive(package, &version).unwrap().0,
                std_archive(package, &version).unwrap().0
            );
        }
    }

    /// Two packages, two archives. If they hashed the same a lock could not
    /// tell which one changed.
    #[test]
    fn each_package_hashes_to_its_own_digest() {
        let version = Version::new(1, 2, 3);
        let digests: Vec<Digest> = TOOLCHAIN_PACKAGES
            .iter()
            .map(|package| std_archive(package, &version).unwrap().1)
            .collect();
        for (index, digest) in digests.iter().enumerate() {
            assert!(
                !digests[index + 1..].contains(digest),
                "two bundled packages share a digest"
            );
        }
    }
}
