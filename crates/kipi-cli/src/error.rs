//! Exit codes and the CLI's failures (roadmap §7): 0 ok, 1 user error, 2 doctor
//! findings, 3 I/O failure. Every failure prints as `error: …` plus `hint: …`.

use std::process::ExitCode;

use kipi_core::error::{Error, ErrorKind};

use crate::output::color::Painter;

/// Success.
pub const EXIT_OK: u8 = 0;
/// Something the user can fix: bad arguments, an invalid store, an unknown repo.
pub const EXIT_USER: u8 = 1;
/// `doctor` found an error-level problem.
pub const EXIT_FINDINGS: u8 = 2;
/// Reading or writing the filesystem failed.
pub const EXIT_IO: u8 = 3;

/// Why a command failed.
#[derive(Debug)]
pub enum Failure {
    /// An error from kipi-core.
    Core(Error),
    /// A CLI-level user error (unknown repo, not yet implemented, ...).
    User {
        /// What is wrong.
        message: String,
        /// What to do about it.
        hint: String,
    },
    /// Writing output failed.
    Output(std::io::Error),
    /// `doctor` printed this many error-level findings.
    Findings(usize),
}

impl Failure {
    /// A user error with its hint.
    pub fn user(message: impl Into<String>, hint: impl Into<String>) -> Self {
        Failure::User {
            message: message.into(),
            hint: hint.into(),
        }
    }

    /// Printing `what` as JSON failed: a path is not valid UTF-8.
    pub fn json(what: &str, e: serde_json::Error) -> Self {
        Failure::user(
            format!("cannot print {what} as JSON: {e}"),
            "JSON output needs every path to be valid UTF-8; rename the offending directory",
        )
    }

    /// The process exit code, from the core error's kind.
    pub fn exit_code(&self) -> u8 {
        match self {
            Failure::Core(e) => match e.kind() {
                ErrorKind::User => EXIT_USER,
                ErrorKind::Io => EXIT_IO,
            },
            Failure::User { .. } => EXIT_USER,
            Failure::Output(_) => EXIT_IO,
            Failure::Findings(_) => EXIT_FINDINGS,
        }
    }

    /// What went wrong and what to do about it.
    pub fn parts(&self) -> (String, String) {
        match self {
            Failure::Core(e) => (e.to_string(), e.hint()),
            Failure::User { message, hint } => (message.clone(), hint.clone()),
            Failure::Output(e) => (
                format!("cannot write output: {e}"),
                "check that stdout is still open (a closed pipe stops kipi)".to_owned(),
            ),
            Failure::Findings(n) => (
                format!(
                    "doctor found {n} error-level problem{}",
                    if *n == 1 { "" } else { "s" }
                ),
                "fix each as its hint says, then run `kipi doctor` again".to_owned(),
            ),
        }
    }

    /// `error: …` and `hint: …`, as printed on stderr.
    pub fn render(&self, paint: &Painter) -> String {
        let (message, hint) = self.parts();
        format!(
            "{} {message}\n  {} {hint}\n",
            paint.error("error:"),
            paint.hint("hint:")
        )
    }
}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        Failure::Core(e)
    }
}

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        Failure::Output(e)
    }
}

/// `code` as an [`ExitCode`].
pub fn exit(code: u8) -> ExitCode {
    ExitCode::from(code)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use kipi_core::error::Location;

    use super::*;

    #[test]
    fn exit_codes_follow_the_error_kind() {
        let io = Error::io(Path::new("x"), std::io::Error::other("boom"), "retry");
        assert_eq!(Failure::from(io).exit_code(), EXIT_IO);
        let invalid = Error::invalid(Location::file("x"), "bad", "fix it");
        assert_eq!(Failure::from(invalid).exit_code(), EXIT_USER);
        assert_eq!(Failure::user("m", "h").exit_code(), EXIT_USER);
    }

    #[test]
    fn renders_error_and_hint() {
        let out = Failure::user("no repo `x`", "run `kipi resolve`").render(&Painter::plain());
        assert_eq!(out, "error: no repo `x`\n  hint: run `kipi resolve`\n");
    }
}
