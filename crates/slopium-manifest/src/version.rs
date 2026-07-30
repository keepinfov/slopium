//! Semantic versions and the requirement syntax `D-036` settles on.
//!
//! Requirements are `^`, `~`, `=`, `>=`, `>`, `<=`, `<`, comma-joined, and a
//! bare `1.2.3` means `^1.2.3`. Selection takes the highest compatible version;
//! that part lives in `resolve`, not here.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::Ordering;
use std::fmt;

/// A semantic version.
///
/// Build metadata is parsed and printed but never compared, which is what
/// semver 2.0 requires.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Option<String>,
    pub build: Option<String>,
}

impl Version {
    pub fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
            pre: None,
            build: None,
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let invalid = || format!("invalid version `{text}`; expected `major.minor.patch`");

        let (rest, build) = match text.split_once('+') {
            Some((rest, build)) => {
                check_identifiers(build, text, "build metadata")?;
                (rest, Some(build.to_owned()))
            }
            None => (text, None),
        };
        let (core, pre) = match rest.split_once('-') {
            Some((core, pre)) => {
                check_identifiers(pre, text, "pre-release")?;
                (core, Some(pre.to_owned()))
            }
            None => (rest, None),
        };

        let mut fields = core.split('.');
        let number = |field: Option<&str>| -> Result<u64, String> {
            let field = field.ok_or_else(invalid)?;
            if field.is_empty() || (field.len() > 1 && field.starts_with('0')) {
                return Err(invalid());
            }
            field.parse::<u64>().map_err(|_| invalid())
        };
        let major = number(fields.next())?;
        let minor = number(fields.next())?;
        let patch = number(fields.next())?;
        if fields.next().is_some() {
            return Err(invalid());
        }

        Ok(Self {
            major,
            minor,
            patch,
            pre,
            build,
        })
    }

    /// Whether this version is a pre-release, which resolution skips unless a
    /// requirement asks for one at the same `major.minor.patch`.
    pub fn is_prerelease(&self) -> bool {
        self.pre.is_some()
    }

    fn core(&self) -> (u64, u64, u64) {
        (self.major, self.minor, self.patch)
    }
}

fn check_identifiers(text: &str, whole: &str, what: &str) -> Result<(), String> {
    if text.is_empty() || text.split('.').any(|part| part.is_empty()) {
        return Err(format!(
            "invalid version `{whole}`; empty {what} identifier"
        ));
    }
    let valid = text
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-'));
    if valid {
        Ok(())
    } else {
        Err(format!("invalid version `{whole}`; malformed {what}"))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core().cmp(&other.core()).then_with(|| {
            match (self.pre.as_deref(), other.pre.as_deref()) {
                // A release outranks any pre-release of the same core version.
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(left), Some(right)) => compare_prerelease(left, right),
            }
        })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Dot-separated identifiers compare left to right; numeric ones numerically
/// and below alphanumeric ones, and a longer run of equal identifiers wins.
fn compare_prerelease(left: &str, right: &str) -> Ordering {
    let mut left = left.split('.');
    let mut right = right.split('.');
    loop {
        return match (left.next(), right.next()) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
            (Some(one), Some(other)) => {
                let ordering = match (one.parse::<u64>(), other.parse::<u64>()) {
                    (Ok(one), Ok(other)) => one.cmp(&other),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => one.cmp(other),
                };
                if ordering == Ordering::Equal {
                    continue;
                }
                ordering
            }
        };
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre {
            write!(formatter, "-{pre}")?;
        }
        if let Some(build) = &self.build {
            write!(formatter, "+{build}")?;
        }
        Ok(())
    }
}

impl Serialize for Version {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Version {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Version::parse(&text).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Op {
    Caret,
    Tilde,
    Exact,
    Greater,
    GreaterEq,
    Less,
    LessEq,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Comparator {
    op: Op,
    version: Version,
    /// How many of `major.minor.patch` the requirement actually wrote. `^1.2`
    /// and `^1.2.0` do not mean the same thing.
    fields: usize,
}

impl Comparator {
    fn matches(&self, candidate: &Version) -> bool {
        let bound = &self.version;
        match self.op {
            Op::Exact => match self.fields {
                1 => candidate.major == bound.major,
                2 => candidate.major == bound.major && candidate.minor == bound.minor,
                _ => candidate.core() == bound.core() && candidate.pre == bound.pre,
            },
            Op::Greater => candidate > bound,
            Op::GreaterEq => candidate >= bound,
            Op::Less => candidate < bound,
            Op::LessEq => candidate <= bound,
            Op::Caret => {
                if candidate < bound {
                    return false;
                }
                // The leftmost non-zero field is the one held fixed: `^0.2.3`
                // allows `0.2.9` but not `0.3.0`.
                if bound.major > 0 || self.fields == 1 {
                    candidate.major == bound.major
                } else if bound.minor > 0 || self.fields == 2 {
                    candidate.major == 0 && candidate.minor == bound.minor
                } else {
                    candidate.core() == bound.core()
                }
            }
            Op::Tilde => {
                if candidate < bound {
                    return false;
                }
                if self.fields == 1 {
                    candidate.major == bound.major
                } else {
                    candidate.major == bound.major && candidate.minor == bound.minor
                }
            }
        }
    }
}

/// A comma-joined conjunction of comparators.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VersionReq {
    comparators: Vec<Comparator>,
    text: String,
}

impl VersionReq {
    /// Matches every release version. Used for a dependency that names a source
    /// but no version, which is legal for `path` and, later, `git`.
    pub fn any() -> Self {
        Self {
            comparators: Vec::new(),
            text: "*".to_owned(),
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let trimmed = text.trim();
        if trimmed == "*" {
            return Ok(Self::any());
        }
        let mut comparators = Vec::new();
        for piece in trimmed.split(',') {
            let piece = piece.trim();
            if piece.is_empty() {
                return Err(format!("invalid requirement `{text}`; empty comparator"));
            }
            // Longest operator first, or `>=` reads as `>`.
            let (op, rest) = if let Some(rest) = piece.strip_prefix(">=") {
                (Op::GreaterEq, rest)
            } else if let Some(rest) = piece.strip_prefix("<=") {
                (Op::LessEq, rest)
            } else if let Some(rest) = piece.strip_prefix('^') {
                (Op::Caret, rest)
            } else if let Some(rest) = piece.strip_prefix('~') {
                (Op::Tilde, rest)
            } else if let Some(rest) = piece.strip_prefix('=') {
                (Op::Exact, rest)
            } else if let Some(rest) = piece.strip_prefix('>') {
                (Op::Greater, rest)
            } else if let Some(rest) = piece.strip_prefix('<') {
                (Op::Less, rest)
            } else {
                // `D-036`: a bare version is a caret requirement.
                (Op::Caret, piece)
            };
            let (version, fields) = parse_partial(rest.trim(), text)?;
            comparators.push(Comparator {
                op,
                version,
                fields,
            });
        }
        Ok(Self {
            comparators,
            text: trimmed.to_owned(),
        })
    }

    pub fn matches(&self, candidate: &Version) -> bool {
        if !self.comparators.iter().all(|one| one.matches(candidate)) {
            return false;
        }
        // A pre-release is only ever offered to a requirement that named a
        // pre-release at the same core version. Otherwise `^1.0.0` would start
        // selecting `2.0.0-alpha`, which nobody means by it.
        if candidate.is_prerelease() {
            return self.comparators.iter().any(|one| {
                one.version.is_prerelease()
                    && one.version.core() == (candidate.major, candidate.minor, candidate.patch)
            });
        }
        true
    }
}

/// A requirement may write fewer than three fields; the missing ones are zero,
/// and the count is kept because it changes what the operator means.
fn parse_partial(text: &str, whole: &str) -> Result<(Version, usize), String> {
    let invalid = || format!("invalid requirement `{whole}`");
    let (core, suffix) = match text.find(['-', '+']) {
        Some(index) => text.split_at(index),
        None => (text, ""),
    };
    let fields = core.split('.').count();
    if fields == 0 || fields > 3 || core.is_empty() {
        return Err(invalid());
    }
    let mut padded = core.to_owned();
    for _ in fields..3 {
        padded.push_str(".0");
    }
    padded.push_str(suffix);
    Ok((Version::parse(&padded).map_err(|_| invalid())?, fields))
}

impl fmt::Display for VersionReq {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

impl Serialize for VersionReq {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.text)
    }
}

impl<'de> Deserialize<'de> for VersionReq {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        VersionReq::parse(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn parses_and_prints_versions() {
        assert_eq!(version("1.2.3"), Version::new(1, 2, 3));
        assert_eq!(version("1.2.3-alpha.1").pre.as_deref(), Some("alpha.1"));
        assert_eq!(version("1.2.3+build.5").build.as_deref(), Some("build.5"));
        for text in ["1.2.3", "0.0.1", "1.2.3-rc.1", "1.2.3-rc.1+meta"] {
            assert_eq!(version(text).to_string(), text);
        }
    }

    #[test]
    fn rejects_malformed_versions() {
        for text in ["1.2", "1.2.3.4", "1.2.x", "01.2.3", "", "1.2.3-", "1.2.3-@"] {
            assert!(Version::parse(text).is_err(), "expected `{text}` to fail");
        }
    }

    #[test]
    fn orders_prereleases_below_their_release() {
        assert!(version("1.0.0-alpha") < version("1.0.0"));
        assert!(version("1.0.0-alpha") < version("1.0.0-alpha.1"));
        assert!(version("1.0.0-alpha.1") < version("1.0.0-alpha.beta"));
        assert!(version("1.0.0-beta") < version("1.0.0-beta.2"));
        assert!(version("1.0.0-beta.2") < version("1.0.0-beta.11"));
        assert!(version("1.0.0-rc.1") < version("1.0.0"));
        assert!(version("1.0.0") < version("1.0.1"));
    }

    #[test]
    fn build_metadata_does_not_affect_ordering() {
        assert_eq!(version("1.0.0+a").cmp(&version("1.0.0+b")), Ordering::Equal);
    }

    #[test]
    fn a_bare_requirement_is_a_caret_requirement() {
        let bare = VersionReq::parse("1.2.3").unwrap();
        assert!(bare.matches(&version("1.2.3")));
        assert!(bare.matches(&version("1.9.0")));
        assert!(!bare.matches(&version("2.0.0")));
        assert!(!bare.matches(&version("1.2.2")));
    }

    #[test]
    fn caret_holds_the_leftmost_non_zero_field() {
        let zero_minor = VersionReq::parse("^0.2.3").unwrap();
        assert!(zero_minor.matches(&version("0.2.9")));
        assert!(!zero_minor.matches(&version("0.3.0")));

        let zero_patch = VersionReq::parse("^0.0.3").unwrap();
        assert!(zero_patch.matches(&version("0.0.3")));
        assert!(!zero_patch.matches(&version("0.0.4")));

        let partial = VersionReq::parse("^1.2").unwrap();
        assert!(partial.matches(&version("1.9.9")));
        assert!(!partial.matches(&version("2.0.0")));
    }

    #[test]
    fn tilde_holds_the_minor_when_one_is_written() {
        let request = VersionReq::parse("~1.2.3").unwrap();
        assert!(request.matches(&version("1.2.9")));
        assert!(!request.matches(&version("1.3.0")));

        let major_only = VersionReq::parse("~1").unwrap();
        assert!(major_only.matches(&version("1.9.0")));
        assert!(!major_only.matches(&version("2.0.0")));
    }

    #[test]
    fn ranges_conjoin() {
        let request = VersionReq::parse(">=1.2.0, <1.5.0").unwrap();
        assert!(request.matches(&version("1.2.0")));
        assert!(request.matches(&version("1.4.9")));
        assert!(!request.matches(&version("1.5.0")));
        assert!(!request.matches(&version("1.1.9")));
    }

    #[test]
    fn exact_respects_how_many_fields_were_written() {
        assert!(VersionReq::parse("=1.2.3")
            .unwrap()
            .matches(&version("1.2.3")));
        assert!(!VersionReq::parse("=1.2.3")
            .unwrap()
            .matches(&version("1.2.4")));
        assert!(VersionReq::parse("=1.2")
            .unwrap()
            .matches(&version("1.2.7")));
        assert!(VersionReq::parse("=1").unwrap().matches(&version("1.7.7")));
    }

    #[test]
    fn prereleases_are_offered_only_when_asked_for() {
        let plain = VersionReq::parse("^1.0.0").unwrap();
        assert!(!plain.matches(&version("1.1.0-alpha")));
        assert!(plain.matches(&version("1.1.0")));

        let asked = VersionReq::parse("^1.1.0-alpha").unwrap();
        assert!(asked.matches(&version("1.1.0-alpha.2")));
        assert!(asked.matches(&version("1.1.0")));
        assert!(!asked.matches(&version("1.2.0-beta")));
    }

    #[test]
    fn any_matches_every_release() {
        let any = VersionReq::any();
        assert!(any.matches(&version("0.0.1")));
        assert!(any.matches(&version("9.9.9")));
        assert!(!any.matches(&version("1.0.0-rc")));
        assert_eq!(any.to_string(), "*");
    }
}
