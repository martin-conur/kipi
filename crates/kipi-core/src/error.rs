//! Error types (thiserror). Every user-facing error carries a hint.
//!
//! Errors about a file point at it as `path:line` ([`Location`]); [`Error::hint`] says
//! what to do about it, and [`Error::report`] renders both the way the CLI prints them.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

/// `Result` with kipi's [`Error`].
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// A position in a file kipi reads, displayed as `path:line`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// The file.
    pub file: PathBuf,
    /// 1-based line, when known.
    pub line: Option<usize>,
}

impl Location {
    /// A whole file, no line.
    pub fn file(file: impl Into<PathBuf>) -> Self {
        Location {
            file: file.into(),
            line: None,
        }
    }

    /// A line in a file.
    pub fn at(file: impl Into<PathBuf>, line: usize) -> Self {
        Location {
            file: file.into(),
            line: Some(line),
        }
    }

    /// `file:line` when `line` is known, else the whole file.
    pub fn maybe_at(file: impl Into<PathBuf>, line: Option<usize>) -> Self {
        Location {
            file: file.into(),
            line,
        }
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(line) => write!(f, "{}:{line}", self.file.display()),
            None => write!(f, "{}", self.file.display()),
        }
    }
}

/// Everything that can go wrong in kipi-core.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A file kipi reads is malformed or fails validation.
    #[error("{at}: {message}")]
    Invalid {
        /// Where.
        at: Location,
        /// What is wrong.
        message: String,
        /// What to do about it.
        hint: String,
    },
    /// A key that the schema does not define (typically a typo).
    #[error("{at}: unknown key `{key}` in {table}")]
    UnknownKey {
        /// Where.
        at: Location,
        /// The offending key.
        key: String,
        /// The table holding it, e.g. `[scopes.acme]`.
        table: String,
        /// Every valid key in that table.
        expected: Vec<String>,
    },
    /// Reading a file failed.
    #[error("{}: {source}", path.display())]
    Io {
        /// The file or directory.
        path: PathBuf,
        /// The underlying error.
        #[source]
        source: io::Error,
        /// What to do about it.
        hint: String,
    },
    /// Neither `$HOME` nor the platform reports a home directory.
    #[error("cannot determine the home directory")]
    NoHome,
}

impl Error {
    /// An [`Error::Invalid`].
    pub fn invalid(at: Location, message: impl Into<String>, hint: impl Into<String>) -> Self {
        Error::Invalid {
            at,
            message: message.into(),
            hint: hint.into(),
        }
    }

    /// An [`Error::Io`] for `path`.
    pub fn io(path: &Path, source: io::Error, hint: impl Into<String>) -> Self {
        Error::Io {
            path: path.to_path_buf(),
            source,
            hint: hint.into(),
        }
    }

    /// What the user should do about this error.
    pub fn hint(&self) -> String {
        match self {
            Error::Invalid { hint, .. } | Error::Io { hint, .. } => hint.clone(),
            Error::UnknownKey { expected, .. } => {
                let valid = expected
                    .iter()
                    .map(|k| format!("`{k}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                match self.suggestion() {
                    Some(s) => format!("did you mean `{s}`? Valid keys: {valid}"),
                    None => format!("valid keys: {valid}"),
                }
            }
            Error::NoHome => "set $HOME".to_owned(),
        }
    }

    /// For an unknown key, the valid key it most likely meant.
    pub fn suggestion(&self) -> Option<&str> {
        match self {
            Error::UnknownKey { key, expected, .. } => {
                closest(key, expected.iter().map(String::as_str))
            }
            _ => None,
        }
    }

    /// The file (and line) this error points at, if any.
    pub fn location(&self) -> Option<&Location> {
        match self {
            Error::Invalid { at, .. } | Error::UnknownKey { at, .. } => Some(at),
            Error::Io { .. } | Error::NoHome => None,
        }
    }

    /// The error and its hint, as the CLI prints them.
    pub fn report(&self) -> String {
        format!("error: {self}\n  hint: {}", self.hint())
    }
}

/// The candidate closest to `input` by edit distance, if it is close enough to be a
/// plausible typo (at most a third of the input's length, minimum 1, maximum 3 edits).
pub fn closest<'a>(input: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let limit = (input.chars().count() / 3).clamp(1, 3);
    candidates
        .into_iter()
        .map(|c| (edit_distance(input, c), c))
        .filter(|(d, _)| *d <= limit)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

/// Edit distance over chars (optimal string alignment: insertions, deletions,
/// substitutions and adjacent transpositions each cost 1).
fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut d = vec![vec![0; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut best = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                best = best.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = best;
        }
    }
    d[a.len()][b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_distance_basics() {
        assert_eq!(edit_distance("skils", "skills"), 1);
        assert_eq!(edit_distance("", "abc"), 3);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
        assert_eq!(edit_distance("same", "same"), 0);
        assert_eq!(edit_distance("alpah", "alpha"), 1);
    }

    #[test]
    fn closest_picks_plausible_typos_only() {
        let keys = ["roots", "skills", "always", "registry"];
        assert_eq!(closest("skils", keys), Some("skills"));
        assert_eq!(closest("regsitry", keys), Some("registry"));
        assert_eq!(closest("colour", keys), None);
        assert_eq!(closest("x", keys), None);
    }

    #[test]
    fn location_display() {
        assert_eq!(Location::at("a/b.toml", 3).to_string(), "a/b.toml:3");
        assert_eq!(Location::file("a/b.toml").to_string(), "a/b.toml");
    }

    #[test]
    fn unknown_key_hint_names_suggestion_and_valid_keys() {
        let err = Error::UnknownKey {
            at: Location::at("bindings.toml", 7),
            key: "skils".into(),
            table: "[scopes.acme]".into(),
            expected: vec!["roots".into(), "skills".into()],
        };
        assert_eq!(
            err.report(),
            "error: bindings.toml:7: unknown key `skils` in [scopes.acme]\n  \
             hint: did you mean `skills`? Valid keys: `roots`, `skills`"
        );
    }
}
