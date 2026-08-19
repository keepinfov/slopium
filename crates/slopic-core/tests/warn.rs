//! Snapshots for a program that compiles and is warned about (`D-122`).
//!
//! The mirror of `compile_fail`, and deliberately its twin down to the
//! `SLOPIUM_UPDATE_SNAPSHOTS` switch: a warning is a diagnostic like any other
//! and its code, its span and its rendering are as much of a contract as an
//! error's. What the two cannot share is the assertion at the top — a fixture
//! here must *compile*, which is the only thing that makes it a warning.

use serde::{Deserialize, Serialize};
use slopic_core::analysis::analyze_source;
use slopic_core::CompileOptions;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ExpectedDiagnostic {
    code: String,
    start: usize,
    end: usize,
    line: usize,
    column: usize,
}

fn fixture_paths() -> Vec<PathBuf> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/warn");
    let mut paths = fs::read_dir(directory)
        .expect("warning fixture directory exists")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "slp"))
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

#[test]
fn warning_fixtures_compile_and_match_codes_spans_and_rendering() {
    let update = std::env::var_os("SLOPIUM_UPDATE_SNAPSHOTS").is_some();
    for source_path in fixture_paths() {
        let source = fs::read_to_string(&source_path).expect("fixture source is readable");
        let name = source_path
            .file_name()
            .expect("fixture has a name")
            .to_string_lossy();
        let analysis = analyze_source(&name, &source, &CompileOptions::default());
        assert!(
            analysis.program.is_some(),
            "warning fixture must compile: {}",
            source_path.display()
        );
        let warnings = analysis.diagnostics;
        assert!(
            !warnings.is_empty(),
            "warning fixture warns about nothing: {}",
            source_path.display()
        );
        let expected = warnings
            .iter()
            .map(|diagnostic| ExpectedDiagnostic {
                code: diagnostic.code.clone(),
                start: diagnostic.span.start,
                end: diagnostic.span.end,
                line: diagnostic.span.line,
                column: diagnostic.span.column,
            })
            .collect::<Vec<_>>();
        let json = format!(
            "{}\n",
            serde_json::to_string_pretty(&expected).expect("snapshot serializes")
        );
        let stderr = format!(
            "{}\n",
            warnings
                .iter()
                .map(|diagnostic| diagnostic.render(&source))
                .collect::<Vec<_>>()
                .join("\n\n")
        );
        let json_path = source_path.with_extension("expect.json");
        let stderr_path = source_path.with_extension("stderr");
        if update {
            fs::write(&json_path, &json).expect("JSON snapshot can be updated");
            fs::write(&stderr_path, &stderr).expect("stderr snapshot can be updated");
        } else {
            assert_eq!(
                fs::read_to_string(&json_path).expect("JSON snapshot exists"),
                json,
                "codes/spans changed for {}",
                source_path.display()
            );
            assert_eq!(
                fs::read_to_string(&stderr_path).expect("stderr snapshot exists"),
                stderr,
                "rendering changed for {}",
                source_path.display()
            );
        }
    }
}
