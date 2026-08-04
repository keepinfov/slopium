//! The library bundled with the compiler: its sources, its module names, and
//! the language items it supplies.
//!
//! It lives here, in a crate with no dependencies, because two consumers must
//! agree about it exactly. The compiler hands these sources to name resolution
//! when a build asks for the toolchain library; the manager hashes them into
//! the archive a lockfile records. When those two disagreed the disagreement
//! was silent — the manager would lock a digest for a library the compiler
//! never saw (`D-076`).
//!
//! The sources themselves are ordinary `.slp` files under `std/` at the root of
//! the repository, the way the runtime is an ordinary `.c` file under
//! `runtime/`. A library written in the language it is for should be readable
//! as the language it is for.

/// The package name the bundled library resolves under, and therefore the
/// namespace its modules take.
pub const STD_PACKAGE: &str = "std";

/// Module name and source for each module of the bundled library, in the order
/// a build sees them.
pub const STD_MODULES: &[(&str, &str)] = &[
    ("option", include_str!("../../../std/option.slp")),
    ("result", include_str!("../../../std/result.slp")),
    ("io", include_str!("../../../std/io.slp")),
    ("process", include_str!("../../../std/process.slp")),
];

/// The language items the bundled library supplies, as a name and a path within
/// the package.
pub const STD_LANGUAGE_ITEMS: &[(&str, &str)] = &[
    ("option", "option:Option"),
    ("result", "result:Result"),
    ("result-ok", "result:Ok"),
    ("result-err", "result:Err"),
];

/// The language items, already namespaced — the form both the manager's
/// resolver and the compiler's defaults want.
pub fn std_language_items() -> Vec<(String, String)> {
    STD_LANGUAGE_ITEMS
        .iter()
        .map(|(name, path)| ((*name).to_owned(), format!("{STD_PACKAGE}:{path}")))
        .collect()
}

/// The path reported for a bundled module in diagnostics. It is not a real
/// file, and the angle brackets keep it from looking like one.
pub fn std_module_path(module: &str) -> String {
    format!("<toolchain>/{STD_PACKAGE}/{module}.slp")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_items_name_modules_that_exist() {
        for (_, path) in STD_LANGUAGE_ITEMS {
            let module = path.split(':').next().expect("a module-qualified path");
            assert!(
                STD_MODULES.iter().any(|(name, _)| name == &module),
                "language item names missing module `{module}`"
            );
        }
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
