//! Sync commands (`on_sync`, roadmap §6): a store-declared program `kipi sync` runs in a
//! repo after materializing it, with the repo's attributes filled into its arguments.
//!
//! A `run` string is split into words here, never by a shell: quotes group, a backslash
//! escapes, and `$VAR`, `~`, globs, `;` and `|` are plain characters. Each word is then a
//! template whose only placeholder is `{attr.<key>}`; filling one replaces it inside its
//! own word, so a value is always exactly part of one argument, whatever it contains.
//!
//! [`State`] is `.kipi/on_sync.toml` in each working tree: per command, the hash of the
//! argv it last ran and how that went, so a second sync is a no-op. It holds hashes and
//! results only; the commands themselves come from store config alone.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::apply::{self, ON_SYNC};
use crate::error::{Error, Location, Result};
use crate::manifest;
use crate::model::check_attribute_key;

/// The hint for a malformed placeholder.
pub const PLACEHOLDER_HINT: &str =
    "the only placeholder is `{attr.<key>}`; write `{{` and `}}` for literal braces";

/// One piece of a [`Word`] template.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Part {
    /// Literal text.
    Lit(String),
    /// `{attr.<key>}`: the repo's resolved attribute `key`.
    Attr(String),
}

/// One argv element as written: literal text and `{attr.*}` placeholders.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Word(pub Vec<Part>);

impl Word {
    /// Parse a word's placeholders: `{attr.<key>}`, with `{{` and `}}` as literal braces.
    pub fn parse(text: &str) -> std::result::Result<Word, String> {
        let mut parts = Vec::new();
        let mut lit = String::new();
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '{' if chars.peek() == Some(&'{') => {
                    chars.next();
                    lit.push('{');
                }
                '}' if chars.peek() == Some(&'}') => {
                    chars.next();
                    lit.push('}');
                }
                '}' => return Err(format!("`{text}` has a lone `}}`")),
                '{' => {
                    let mut inner = String::new();
                    let mut closed = false;
                    for c in chars.by_ref() {
                        if c == '}' {
                            closed = true;
                            break;
                        }
                        inner.push(c);
                    }
                    if !closed {
                        return Err(format!("`{text}` has an unclosed `{{`"));
                    }
                    let Some(key) = inner.strip_prefix("attr.") else {
                        return Err(format!("`{text}` has an unknown placeholder `{{{inner}}}`"));
                    };
                    check_attribute_key(key).map_err(|why| {
                        format!("the placeholder `{{{inner}}}` in `{text}`: attribute key {why}")
                    })?;
                    if !lit.is_empty() {
                        parts.push(Part::Lit(std::mem::take(&mut lit)));
                    }
                    parts.push(Part::Attr(key.to_owned()));
                }
                c => lit.push(c),
            }
        }
        if !lit.is_empty() || parts.is_empty() {
            parts.push(Part::Lit(lit));
        }
        Ok(Word(parts))
    }

    /// The attribute keys this word reads, in order.
    pub fn attrs(&self) -> impl Iterator<Item = &str> {
        self.0.iter().filter_map(|p| match p {
            Part::Attr(k) => Some(k.as_str()),
            Part::Lit(_) => None,
        })
    }

    /// The word with every placeholder filled by `lookup`; the first key it has no value
    /// for is the error.
    pub fn fill<'a>(
        &self,
        lookup: impl Fn(&str) -> Option<&'a str>,
    ) -> std::result::Result<String, String> {
        let mut out = String::new();
        for part in &self.0 {
            match part {
                Part::Lit(s) => out.push_str(s),
                Part::Attr(k) => out.push_str(lookup(k).ok_or_else(|| k.clone())?),
            }
        }
        Ok(out)
    }
}

/// The word as written: literal braces doubled, placeholders as `{attr.<key>}`.
impl fmt::Display for Word {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for part in &self.0 {
            match part {
                Part::Lit(s) => f.write_str(&s.replace('{', "{{").replace('}', "}}"))?,
                Part::Attr(k) => write!(f, "{{attr.{k}}}")?,
            }
        }
        Ok(())
    }
}

/// Split a `run` string into words, without a shell: unquoted whitespace separates,
/// `'…'` is literal, `"…"` groups (`\"` and `\\` escape inside it), and a bare `\x` is `x`.
pub fn split(run: &str) -> std::result::Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut chars = run.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => words.extend(word.take()),
            '\'' => {
                let w = word.get_or_insert_with(String::new);
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => w.push(c),
                        None => return Err("a `'` quote is not closed".to_owned()),
                    }
                }
            }
            '"' => {
                let w = word.get_or_insert_with(String::new);
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c @ ('"' | '\\')) => w.push(c),
                            Some(c) => {
                                w.push('\\');
                                w.push(c);
                            }
                            None => return Err("a `\"` quote is not closed".to_owned()),
                        },
                        Some(c) => w.push(c),
                        None => return Err("a `\"` quote is not closed".to_owned()),
                    }
                }
            }
            '\\' => match chars.next() {
                Some(c) => word.get_or_insert_with(String::new).push(c),
                None => return Err("it ends with a lone `\\`".to_owned()),
            },
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    words.extend(word);
    Ok(words)
}

/// Why a `run` is not a valid template, with what to do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateError {
    /// What is wrong.
    pub message: String,
    /// What to do.
    pub hint: &'static str,
}

/// Parse `words` (split from a `run` string, or its array form) into templates. Empty
/// argv, a bad placeholder, or a placeholder in the program word are errors: an attribute
/// value never chooses the program.
pub fn template(words: &[String]) -> std::result::Result<Vec<Word>, TemplateError> {
    if words.is_empty() {
        return Err(TemplateError {
            message: "`run` is empty".to_owned(),
            hint: "write the program and its arguments, e.g. `run = \"tool --flag\"`",
        });
    }
    let argv = words
        .iter()
        .map(|w| Word::parse(w))
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|message| TemplateError {
            message,
            hint: PLACEHOLDER_HINT,
        })?;
    if argv[0].attrs().next().is_some() {
        return Err(TemplateError {
            message: format!(
                "the program `{}` contains a placeholder; an attribute may fill arguments, \
                 never choose the program",
                argv[0]
            ),
            hint: "name the program literally; pass the attribute as an argument instead",
        });
    }
    Ok(argv)
}

/// `argv` with every placeholder filled by `lookup`; the first missing key is the error.
pub fn fill<'a>(
    argv: &[Word],
    lookup: impl Fn(&str) -> Option<&'a str> + Copy,
) -> std::result::Result<Vec<String>, String> {
    argv.iter().map(|w| w.fill(lookup)).collect()
}

/// The hash `.kipi/on_sync.toml` records for a filled argv.
pub fn argv_hash(argv: &[String]) -> String {
    manifest::hash(&argv.join("\0"))
}

/// The key `machine.toml`'s `approved_commands` holds for a command: a hash over its
/// store, source rule, name, `worktrees` and unfilled template, so a changed `run` or the
/// same command from elsewhere needs approving again, and a changed attribute does not.
pub fn approval_key(store: &str, rule: &str, name: &str, argv: &[Word], worktrees: bool) -> String {
    let mut fields = vec![
        store.to_owned(),
        rule.to_owned(),
        name.to_owned(),
        worktrees.to_string(),
    ];
    fields.extend(argv.iter().map(Word::to_string));
    manifest::hash(&fields.join("\0"))
}

/// `argv` for display, each word shell-quoted when it needs it.
pub fn display(argv: &[impl AsRef<str>]) -> String {
    argv.iter()
        .map(|w| match w.as_ref() {
            "" => "''".to_owned(),
            w => crate::scan::shell_word(w),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The `.kipi/on_sync.toml` schema version this kipi reads and writes.
pub const STATE_VERSION: u32 = 1;

/// `.kipi/on_sync.toml`: the last run of each sync command in one working tree.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// Schema version ([`STATE_VERSION`]).
    pub version: u32,
    /// One per command, by name.
    #[serde(default, rename = "run", skip_serializing_if = "Vec::is_empty")]
    pub runs: Vec<Record>,
}

/// The last run of one command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The command's name.
    pub name: String,
    /// [`argv_hash`] of what ran.
    pub argv_hash: String,
    /// It exited 0.
    pub ok: bool,
    /// The exit code; absent when killed by a signal or it could not be started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
    /// When it ran, in unix seconds.
    pub at: u64,
}

impl State {
    /// The state file of the working tree `dir`.
    pub fn path(dir: &Path) -> PathBuf {
        dir.join(ON_SYNC)
    }

    /// Read the state of `dir`; empty when there is none.
    pub fn load(dir: &Path) -> Result<State> {
        let path = State::path(dir);
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(State::default()),
            Err(e) => return Err(Error::io(&path, e, "check that the file is readable")),
        };
        let hint = "delete it; every sync command then runs again on the next `kipi sync`";
        let state: State = toml_edit::de::from_str(&text).map_err(|e| {
            Error::invalid(
                Location::file(&path),
                format!("malformed sync-command state: {}", e.message()),
                hint,
            )
        })?;
        if state.version != STATE_VERSION {
            return Err(Error::invalid(
                Location::file(&path),
                format!(
                    "sync-command state version {} is not supported (expected {STATE_VERSION})",
                    state.version
                ),
                format!("upgrade kipi, or {hint}"),
            ));
        }
        Ok(state)
    }

    /// The record of command `name`.
    pub fn get(&self, name: &str) -> Option<&Record> {
        self.runs.iter().find(|r| r.name == name)
    }

    /// Replace (or add) the record of `record.name`.
    pub fn set(&mut self, record: Record) {
        match self.runs.iter_mut().find(|r| r.name == record.name) {
            Some(r) => *r = record,
            None => self.runs.push(record),
        }
    }

    /// Write the state of `dir`, or delete the file when it records nothing; then remove
    /// `.kipi/` if that left it empty.
    pub fn save(&self, dir: &Path) -> Result<()> {
        let path = State::path(dir);
        let hint = "check that the directory is writable";
        if self.runs.is_empty() {
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(Error::io(&path, e, hint)),
            }
            if let Some(kipi) = path.parent() {
                let _ = fs::remove_dir(kipi);
            }
            return Ok(());
        }
        let text = toml_edit::ser::to_string_pretty(&State {
            version: STATE_VERSION,
            runs: self.runs.clone(),
        })
        .expect("the state serializes to TOML");
        if fs::read_to_string(&path).ok().as_deref() == Some(text.as_str()) {
            return Ok(());
        }
        if let Some(kipi) = path.parent() {
            fs::create_dir_all(kipi).map_err(|e| Error::io(kipi, e, hint))?;
        }
        apply::write_atomic(&path, &text)
    }
}

/// How one run of a command went.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ran {
    /// It ran to the end: its exit code (`None` when killed by a signal).
    Exited {
        /// The exit code.
        exit: Option<i32>,
        /// It exited 0.
        ok: bool,
    },
    /// It could not be started (program not found, not executable).
    SpawnFailed(String),
}

/// Run `argv` in `dir` with `env` added: stdin closed, its stdout sent to kipi's stderr
/// (so `--json` stays clean), its stderr inherited. A failure is the outcome, not an error.
pub fn spawn(argv: &[String], dir: &Path, env: &[(&str, &str)]) -> Ran {
    let (program, args) = argv.split_first().expect("a template has a program word");
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::from(io::stderr()));
    for (k, v) in env {
        cmd.env(k, v);
    }
    match cmd.status() {
        Ok(status) => Ran::Exited {
            exit: status.code(),
            ok: status.success(),
        },
        Err(e) => Ran::SpawnFailed(format!("cannot run {program}: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &[&str]) -> Vec<String> {
        s.iter().map(|w| (*w).to_owned()).collect()
    }

    #[test]
    fn splits_on_unquoted_whitespace() {
        assert_eq!(split("  a  b\tc ").unwrap(), words(&["a", "b", "c"]));
        assert_eq!(split("").unwrap(), Vec::<String>::new());
        assert_eq!(split("   ").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn quotes_group_and_escape() {
        assert_eq!(split("a 'b c' d").unwrap(), words(&["a", "b c", "d"]));
        assert_eq!(
            split(r#"x "y \"z\" \\ \n""#).unwrap(),
            words(&["x", r#"y "z" \ \n"#])
        );
        assert_eq!(split(r"a\ b \'c").unwrap(), words(&["a b", "'c"]));
        assert_eq!(split("'' x").unwrap(), words(&["", "x"]));
        assert_eq!(split("pre'mid'\"post\"").unwrap(), words(&["premidpost"]));
        assert_eq!(split(r"'it'\''s'").unwrap(), words(&["it's"]));
    }

    #[test]
    fn nothing_is_expanded() {
        assert_eq!(
            split("echo $HOME ~ *.rs ; ls | wc && `id`").unwrap(),
            words(&[
                "echo", "$HOME", "~", "*.rs", ";", "ls", "|", "wc", "&&", "`id`"
            ])
        );
    }

    #[test]
    fn unterminated_quotes_and_trailing_backslash_fail() {
        assert!(split("a 'b").unwrap_err().contains("`'`"));
        assert!(split("a \"b").unwrap_err().contains("`\"`"));
        assert!(split("a \"b\\").is_err());
        assert!(split("a \\").unwrap_err().contains("lone"));
    }

    #[test]
    fn placeholders_parse_with_escapes() {
        let w = Word::parse("--key={attr.tracker.project}!").unwrap();
        assert_eq!(
            w.0,
            vec![
                Part::Lit("--key=".into()),
                Part::Attr("tracker.project".into()),
                Part::Lit("!".into())
            ]
        );
        assert_eq!(w.to_string(), "--key={attr.tracker.project}!");
        let braces = Word::parse("{{x}} {{attr.y}}").unwrap();
        assert_eq!(braces.0, vec![Part::Lit("{x} {attr.y}".into())]);
        assert_eq!(braces.to_string(), "{{x}} {{attr.y}}");
        assert_eq!(Word::parse("").unwrap().0, vec![Part::Lit(String::new())]);
    }

    #[test]
    fn bad_placeholders_fail() {
        assert!(
            Word::parse("{x}")
                .unwrap_err()
                .contains("unknown placeholder `{x}`")
        );
        assert!(Word::parse("{attr.a").unwrap_err().contains("unclosed"));
        assert!(Word::parse("a}").unwrap_err().contains("lone"));
        assert!(
            Word::parse("{attr.a..b}")
                .unwrap_err()
                .contains("empty segment")
        );
        assert!(Word::parse("{attr.}").is_err());
    }

    #[test]
    fn the_program_word_takes_no_placeholder() {
        let err = template(&words(&["{attr.tool}", "x"])).unwrap_err();
        assert!(err.message.contains("never choose the program"), "{err:?}");
        assert!(template(&words(&["tool", "{attr.x}"])).is_ok());
        assert!(template(&[]).unwrap_err().message.contains("empty"));
        assert_eq!(
            template(&words(&["tool", "{x}"])).unwrap_err().hint,
            PLACEHOLDER_HINT
        );
    }

    #[test]
    fn filling_keeps_each_value_in_its_word() {
        let argv = template(&words(&["tool", "--k={attr.a}", "{attr.a}{attr.b}"])).unwrap();
        let attrs = [("a", "x y; $(z)"), ("b", "'q'")];
        let lookup = |k: &str| attrs.iter().find(|(n, _)| *n == k).map(|(_, v)| *v);
        assert_eq!(
            fill(&argv, lookup).unwrap(),
            words(&["tool", "--k=x y; $(z)", "x y; $(z)'q'"])
        );
        assert_eq!(fill(&argv, |_| None).unwrap_err(), "a");
    }

    #[test]
    fn approval_keys_ignore_values_but_not_templates() {
        let argv = template(&words(&["tool", "{attr.a}"])).unwrap();
        let other = template(&words(&["tool", "{attr.b}"])).unwrap();
        let key = approval_key("s", "repos.r.on_sync", "n", &argv, false);
        assert_eq!(key, approval_key("s", "repos.r.on_sync", "n", &argv, false));
        assert_ne!(
            key,
            approval_key("s", "repos.r.on_sync", "n", &other, false)
        );
        assert_ne!(key, approval_key("s", "repos.r.on_sync", "n", &argv, true));
        assert_ne!(key, approval_key("t", "repos.r.on_sync", "n", &argv, false));
        assert_ne!(
            key,
            approval_key("s", "scopes.r.on_sync", "n", &argv, false)
        );
        assert!(key.starts_with("sha256:"));
    }

    #[test]
    fn display_quotes_words_that_need_it() {
        assert_eq!(display(&["tool", "a b", "it's"]), r"tool 'a b' 'it'\''s'");
    }

    #[test]
    fn state_round_trips_and_empties_away() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(State::load(dir.path()).unwrap(), State::default());
        let mut state = State::default();
        state.set(Record {
            name: "a".into(),
            argv_hash: argv_hash(&words(&["x"])),
            ok: false,
            exit: None,
            at: 7,
        });
        state.save(dir.path()).unwrap();
        let back = State::load(dir.path()).unwrap();
        assert_eq!(back.runs, state.runs);
        assert_eq!(back.version, STATE_VERSION);
        State::default().save(dir.path()).unwrap();
        assert!(!dir.path().join(".kipi").exists());
    }

    #[test]
    fn spawn_reports_exit_codes_and_missing_programs() {
        let dir = tempfile::tempdir().unwrap();
        let ok = spawn(&words(&["true"]), dir.path(), &[]);
        assert_eq!(
            ok,
            Ran::Exited {
                exit: Some(0),
                ok: true
            }
        );
        let failed = spawn(&words(&["false"]), dir.path(), &[]);
        assert_eq!(
            failed,
            Ran::Exited {
                exit: Some(1),
                ok: false
            }
        );
        let missing = spawn(&words(&["kipi-no-such-program"]), dir.path(), &[]);
        assert!(matches!(missing, Ran::SpawnFailed(m) if m.contains("kipi-no-such-program")));
    }
}
