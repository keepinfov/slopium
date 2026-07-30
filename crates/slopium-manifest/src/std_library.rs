//! The library bundled with the compiler, written down once.
//!
//! These two module sources used to exist in three copies — in the compiler
//! library, in the project manager's cache keying, and in the language server —
//! and the language-item defaults in four. Three copies of a source string is
//! three chances for them to disagree about what `std` contains, and nothing
//! would have reported it (`D-034`).

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
