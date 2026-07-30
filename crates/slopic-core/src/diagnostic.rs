use serde::Serialize;
use std::fmt;

pub mod codes {
    pub const UNKNOWN: &str = "SL0000";
    pub const UNKNOWN_ESCAPE: &str = "SL0001";
    pub const UNTERMINATED_STRING: &str = "SL0002";
    pub const UNEXPECTED_CLOSE: &str = "SL0003";
    pub const UNCLOSED_LIST: &str = "SL0004";
    pub const MAX_NESTING: &str = "SL0005";
    pub const INVALID_SYNTAX: &str = "SL0100";
    pub const NAME_OR_TYPE: &str = "SL0200";
    pub const OWNERSHIP: &str = "SL0300";
    pub const MATCH: &str = "SL0400";
    pub const ENTRY_POINT: &str = "SL0401";
    pub const MODULE: &str = "SL0450";
    pub const DEPENDENCY: &str = "SL0451";
    pub const GENERIC: &str = "SL0452";
    pub const STANDARD_LIBRARY: &str = "SL0453";
    pub const UNSUPPORTED_TARGET: &str = "SL0500";
    pub const UNSUPPORTED_ABI: &str = "SL0501";
    pub const INPUT_IO: &str = "SL0600";
    pub const OUTPUT_IO: &str = "SL0601";
    pub const TOOLCHAIN: &str = "SL0602";
    pub const INTERNAL: &str = "SL0700";

    pub const ALL: &[(&str, &str)] = &[
        (UNKNOWN_ESCAPE, "unknown string escape"),
        (UNTERMINATED_STRING, "unterminated string literal"),
        (UNEXPECTED_CLOSE, "unexpected closing parenthesis"),
        (UNCLOSED_LIST, "unclosed list"),
        (MAX_NESTING, "expression nesting is too deep"),
        (INVALID_SYNTAX, "invalid declaration or expression syntax"),
        (NAME_OR_TYPE, "name resolution or type error"),
        (OWNERSHIP, "ownership or borrowing error"),
        (MATCH, "pattern matching error"),
        (ENTRY_POINT, "invalid program entry point"),
        (MODULE, "module resolution error"),
        (DEPENDENCY, "package dependency error"),
        (GENERIC, "generic declaration or instantiation error"),
        (STANDARD_LIBRARY, "standard library contract error"),
        (UNSUPPORTED_TARGET, "unsupported compilation target"),
        (UNSUPPORTED_ABI, "unsupported target ABI operation"),
        (INPUT_IO, "compiler input error"),
        (OUTPUT_IO, "compiler output error"),
        (TOOLCHAIN, "external toolchain error"),
        (INTERNAL, "internal compiler error"),
    ];
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub column: usize,
}

impl Span {
    pub fn join(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
            line: self.line,
            column: self.column,
        }
    }
}

/// Which source file each span came from.
///
/// Package analysis merges every module into one virtual source and shifts
/// spans by that module's offset into it, so `start` names the file while
/// `line` and `column` stay file-local. Diagnostics undo the shift; debug
/// line tables only need the file, which is what this answers.
#[derive(Clone, Debug, Default)]
pub struct SourceMap {
    /// `(base, path)` ascending by base, one entry per file.
    files: Vec<(usize, String)>,
}

impl SourceMap {
    /// The map for a compilation of one file, whose spans are unshifted.
    pub fn single(path: &str) -> Self {
        Self {
            files: vec![(0, path.to_owned())],
        }
    }

    /// Builds a map from `(base, path)` pairs in any order.
    pub fn new(files: impl IntoIterator<Item = (usize, String)>) -> Self {
        let mut files: Vec<(usize, String)> = files.into_iter().collect();
        files.sort_by_key(|(base, _)| *base);
        Self { files }
    }

    /// The paths, in the order [`SourceMap::index_of`] numbers them.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.iter().map(|(_, path)| path.as_str())
    }

    /// Index of the file `span` came from, or `None` when the map is empty.
    ///
    /// The first file always starts at offset 0, so every span of a non-empty
    /// map lands in some file.
    pub fn index_of(&self, span: Span) -> Option<usize> {
        if self.files.is_empty() {
            return None;
        }
        Some(
            self.files
                .partition_point(|(base, _)| *base <= span.start)
                .saturating_sub(1),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiagnosticLabel {
    pub span: Span,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Applicability {
    MachineApplicable,
    MaybeIncorrect,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Suggestion {
    pub span: Span,
    pub replacement: String,
    pub message: String,
    pub applicability: Applicability,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    pub file: String,
    pub span: Span,
    pub help: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<DiagnosticLabel>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub suggestions: Vec<Suggestion>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

impl Diagnostic {
    pub fn error(
        code: impl Into<String>,
        file: impl Into<String>,
        span: Span,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.into(),
            severity: Severity::Error,
            message: message.into(),
            file: file.into(),
            span,
            help: None,
            labels: Vec::new(),
            notes: Vec::new(),
            suggestions: Vec::new(),
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn with_label(mut self, span: Span, message: impl Into<String>) -> Self {
        self.labels.push(DiagnosticLabel {
            span,
            message: message.into(),
        });
        self
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    pub fn with_suggestion(
        mut self,
        span: Span,
        replacement: impl Into<String>,
        message: impl Into<String>,
        applicability: Applicability,
    ) -> Self {
        self.suggestions.push(Suggestion {
            span,
            replacement: replacement.into(),
            message: message.into(),
            applicability,
        });
        self
    }

    pub fn render(&self, source: &str) -> String {
        let line = source
            .lines()
            .nth(self.span.line.saturating_sub(1))
            .unwrap_or("");
        let width = self.span.end.saturating_sub(self.span.start).max(1);
        let marker = format!(
            "{}{}",
            " ".repeat(self.span.column.saturating_sub(1)),
            "^".repeat(width.min(line.len().max(1)))
        );
        let mut rendered = format!(
            "{}:{}:{}: error[{}]: {}\n  |\n{:>2} | {}\n  | {}",
            self.file,
            self.span.line,
            self.span.column,
            self.code,
            self.message,
            self.span.line,
            line,
            marker
        );
        for label in &self.labels {
            rendered.push_str(&format!(
                "\n  = label {}:{}: {}",
                label.span.line, label.span.column, label.message
            ));
        }
        if let Some(help) = &self.help {
            rendered.push_str(&format!("\n  = help: {help}"));
        }
        for note in &self.notes {
            rendered.push_str(&format!("\n  = note: {note}"));
        }
        for suggestion in &self.suggestions {
            rendered.push_str(&format!(
                "\n  = suggestion {}:{}: {}: replace with `{}`",
                suggestion.span.line,
                suggestion.span.column,
                suggestion.message,
                suggestion.replacement
            ));
        }
        rendered
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}:{}: {:?}[{}]: {}",
            self.file, self.span.line, self.span.column, self.severity, self.code, self.message
        )
    }
}

pub type CompileResult<T> = Result<T, Vec<Diagnostic>>;

#[cfg(test)]
mod tests {
    use super::{codes, SourceMap, Span};
    use std::collections::HashSet;

    fn at(start: usize) -> Span {
        Span {
            start,
            end: start,
            line: 1,
            column: 1,
        }
    }

    #[test]
    fn a_span_resolves_to_the_file_whose_range_contains_it() {
        // Three files of four, two and six bytes, laid out the way package
        // analysis lays them out: one separating byte after each.
        let map = SourceMap::new([
            (0, "first.slp".to_owned()),
            (5, "second.slp".to_owned()),
            (8, "third.slp".to_owned()),
        ]);

        for (start, expected) in [(0, 0), (3, 0), (4, 0), (5, 1), (7, 1), (8, 2), (13, 2)] {
            assert_eq!(
                map.index_of(at(start)),
                Some(expected),
                "offset {start} belongs to file {expected}"
            );
        }
    }

    #[test]
    fn the_map_is_built_in_base_order_whatever_order_it_is_given() {
        let map = SourceMap::new([
            (8, "third.slp".to_owned()),
            (0, "first.slp".to_owned()),
            (5, "second.slp".to_owned()),
        ]);

        assert_eq!(
            map.paths().collect::<Vec<_>>(),
            ["first.slp", "second.slp", "third.slp"]
        );
        assert_eq!(map.index_of(at(6)), Some(1));
    }

    #[test]
    fn an_empty_map_resolves_nothing() {
        assert_eq!(SourceMap::default().index_of(at(0)), None);
        assert_eq!(SourceMap::default().paths().count(), 0);
        assert_eq!(SourceMap::single("only.slp").index_of(at(9000)), Some(0));
    }

    #[test]
    fn diagnostic_codes_are_documented_and_unique() {
        let mut seen = HashSet::new();
        for (code, description) in codes::ALL {
            assert!(code.starts_with("SL") && code.len() == 6);
            assert!(!description.is_empty());
            assert!(seen.insert(code), "duplicate diagnostic code {code}");
        }
    }
}
