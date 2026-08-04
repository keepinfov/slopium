//! The library bundled with the compiler: its sources, its module names, and
//! the language items it supplies.
//!
//! It lives here, in a crate with no dependencies, because two consumers must
//! agree about it exactly. The compiler hands these sources to name resolution
//! when a build asks for a toolchain library; the manager hashes them into the
//! archive a lockfile records. When those two disagreed the disagreement was
//! silent — the manager would lock a digest for a library the compiler never
//! saw (`D-076`).
//!
//! The sources themselves are ordinary `.slp` files under `std/` at the root of
//! the repository, the way the runtime is an ordinary `.c` file under
//! `runtime/`. A library written in the language it is for should be readable
//! as the language it is for.
//!
//! There are two packages, not one. `core` is what a freestanding program can
//! have; `std` is what a hosted one adds, and it depends on `core` and
//! re-exports its language items so that exactly one direct dependency ever
//! declares them (`D-082`, and `D-041` for why that matters).

/// One package of the bundled library.
pub struct ToolchainPackage {
    /// The name it resolves under, and therefore the namespace its modules
    /// take.
    pub name: &'static str,
    /// Module name and source, in the order a build sees them. Order is
    /// load-bearing: a module may only take one that came before it.
    pub modules: &'static [(&'static str, &'static str)],
    /// The other toolchain packages it depends on.
    pub dependencies: &'static [&'static str],
    /// The language items it supplies, as a name and a path within itself.
    pub language_items: &'static [(&'static str, &'static str)],
}

/// The package a freestanding program depends on: no `extern`, no runtime call
/// that libc has to be behind.
pub const CORE_PACKAGE: &str = "core";

/// The package a hosted program depends on. It is `core` plus what needs an
/// operating system.
pub const STD_PACKAGE: &str = "std";

/// Every package bundled with the compiler, dependencies before dependents.
pub const TOOLCHAIN_PACKAGES: &[ToolchainPackage] = &[
    ToolchainPackage {
        name: CORE_PACKAGE,
        modules: &[
            ("option", include_str!("../../../std/core/option.slp")),
            ("result", include_str!("../../../std/core/result.slp")),
        ],
        dependencies: &[],
        language_items: &[
            ("option", "option:Option"),
            ("result", "result:Result"),
            ("result-ok", "result:Ok"),
            ("result-err", "result:Err"),
        ],
    },
    ToolchainPackage {
        name: STD_PACKAGE,
        modules: &[
            ("prelude", include_str!("../../../std/std/prelude.slp")),
            ("io", include_str!("../../../std/std/io.slp")),
            ("process", include_str!("../../../std/std/process.slp")),
        ],
        dependencies: &[CORE_PACKAGE],
        // Pointed at `prelude`, which takes them from `core` and exports them
        // again. `std` supplies them without owning them (`D-082`).
        language_items: &[
            ("option", "prelude:Option"),
            ("result", "prelude:Result"),
            ("result-ok", "prelude:Ok"),
            ("result-err", "prelude:Err"),
        ],
    },
];

/// The bundled package of that name, if there is one.
pub fn toolchain_package(name: &str) -> Option<&'static ToolchainPackage> {
    TOOLCHAIN_PACKAGES
        .iter()
        .find(|package| package.name == name)
}

/// The language items a package supplies, already namespaced — the form both
/// the manager's resolver and the compiler's defaults want.
pub fn language_items_of(package: &str) -> Vec<(String, String)> {
    toolchain_package(package)
        .map(|package| {
            package
                .language_items
                .iter()
                .map(|(name, path)| ((*name).to_owned(), format!("{}:{path}", package.name)))
                .collect()
        })
        .unwrap_or_default()
}

/// The path reported for a bundled module in diagnostics. It is not a real
/// file, and the angle brackets keep it from looking like one.
pub fn toolchain_module_path(package: &str, module: &str) -> String {
    format!("<toolchain>/{package}/{module}.slp")
}

/// The path on disk, relative to the repository root, that a bundled module is
/// embedded from. Only the formatting test wants this.
pub fn toolchain_source_path(package: &str, module: &str) -> String {
    format!("std/{package}/{module}.slp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_items_name_modules_that_exist() {
        for package in TOOLCHAIN_PACKAGES {
            for (_, path) in package.language_items {
                let module = path.split(':').next().expect("a module-qualified path");
                assert!(
                    package.modules.iter().any(|(name, _)| name == &module),
                    "`{}` names a language item in missing module `{module}`",
                    package.name
                );
            }
        }
    }

    #[test]
    fn every_module_exports_something() {
        for package in TOOLCHAIN_PACKAGES {
            for (name, source) in package.modules {
                assert!(
                    source.contains("(export"),
                    "module `{}:{name}` exports nothing",
                    package.name
                );
            }
        }
    }

    /// Dependencies come before dependents, because the resolver and the
    /// compiler both walk this table in order and neither sorts it.
    #[test]
    fn dependencies_are_declared_before_they_are_used() {
        let mut seen: Vec<&str> = Vec::new();
        for package in TOOLCHAIN_PACKAGES {
            for dependency in package.dependencies {
                assert!(
                    seen.contains(dependency),
                    "`{}` depends on `{dependency}`, which is not listed before it",
                    package.name
                );
            }
            seen.push(package.name);
        }
    }

    /// Every package that declares language items declares all four, or a root
    /// depending on it alone would have some and not others.
    #[test]
    fn language_items_come_as_a_set() {
        for package in TOOLCHAIN_PACKAGES {
            if package.language_items.is_empty() {
                continue;
            }
            let names: Vec<&str> = package.language_items.iter().map(|(n, _)| *n).collect();
            assert_eq!(
                names,
                vec!["option", "result", "result-ok", "result-err"],
                "`{}` declares an incomplete set of language items",
                package.name
            );
        }
    }
}
