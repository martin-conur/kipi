//! Leak guard: scan files for denylisted patterns and absolute home paths.
//!
//! The denylist is one regex per line (blank lines and `#` comments ignored) from:
//! - `.leak-denylist` at the repo root (committed; generic patterns only),
//! - `$KIPI_LEAK_DENYLIST`: extra patterns, newline-separated, same syntax,
//! - `$KIPI_LEAK_DENYLIST_FILE`: path to a file in the same format.
//!
//! The environment sources are for private additions (an employer's name, internal
//! hostnames) that must never be committed, not even to the denylist itself.
//!
//! Independently of the denylist, any absolute home path (`/Users/<name>`,
//! `/home/<name>`, `C:\Users\<name>`) is a hit unless `<name>` is an obvious
//! placeholder (see [`PLACEHOLDER_USERS`]). Docs should write `~` instead.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

/// Name of the committed denylist file at the repo root.
pub const DENYLIST_FILE: &str = ".leak-denylist";
/// Env var holding extra newline-separated patterns.
pub const ENV_PATTERNS: &str = "KIPI_LEAK_DENYLIST";
/// Env var holding the path of an extra denylist file.
pub const ENV_FILE: &str = "KIPI_LEAK_DENYLIST_FILE";

/// User names that are clearly placeholders, or fixed system accounts no person owns
/// (`/home/linuxbrew`, Homebrew's Linux prefix, which cargo-dist's release workflow
/// uses), and so are allowed after `/Users/` or `/home/`.
pub const PLACEHOLDER_USERS: &[&str] = &[
    "user",
    "username",
    "you",
    "yourname",
    "example",
    "shared",
    "linuxbrew",
];

static HOME_PATH: LazyLock<Regex> = LazyLock::new(|| {
    // The leading group stands in for a lookbehind: the path must not continue a
    // word, a relative path (`./`, `~/`) or a URL host (`example.com/home/...`).
    Regex::new(r"(?:^|[^A-Za-z0-9._~-])(/(?:Users|home)/([A-Za-z0-9._-]+)|[A-Za-z]:\\Users\\([A-Za-z0-9._-]+))")
        .expect("home path regex is valid")
});

/// A compiled set of denylist patterns.
#[derive(Debug, Default)]
pub struct Denylist {
    patterns: Vec<Regex>,
}

impl Denylist {
    /// Parse denylist text; `origin` names the source in error messages.
    pub fn parse(text: &str, origin: &str) -> Result<Self, String> {
        let mut list = Denylist::default();
        list.extend(text, origin)?;
        Ok(list)
    }

    /// Add the patterns in `text` (one per line, `#` comments, blanks skipped).
    pub fn extend(&mut self, text: &str, origin: &str) -> Result<(), String> {
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let re = Regex::new(line)
                .map_err(|e| format!("{origin}:{}: invalid regex `{line}`: {e}", i + 1))?;
            self.patterns.push(re);
        }
        Ok(())
    }

    /// Load `<root>/.leak-denylist` (if present) plus the env sources, read through `env`
    /// so tests can inject values.
    pub fn load(root: &Path, env: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let mut list = Denylist::default();
        let committed = root.join(DENYLIST_FILE);
        if committed.is_file() {
            let text =
                fs::read_to_string(&committed).map_err(|e| format!("{DENYLIST_FILE}: {e}"))?;
            list.extend(&text, DENYLIST_FILE)?;
        }
        if let Some(text) = env(ENV_PATTERNS) {
            list.extend(&text, ENV_PATTERNS)?;
        }
        if let Some(path) = env(ENV_FILE).filter(|p| !p.is_empty()) {
            let text = fs::read_to_string(&path).map_err(|e| format!("{ENV_FILE}={path}: {e}"))?;
            list.extend(&text, ENV_FILE)?;
        }
        Ok(list)
    }

    /// Number of patterns loaded.
    pub fn len(&self) -> usize {
        self.patterns.len()
    }

    /// True when no patterns are loaded (home paths are still checked).
    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }
}

/// Why a line was flagged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HitKind {
    /// Matched a denylist pattern (the pattern's source text).
    Denylisted(String),
    /// Contains an absolute home path.
    HomePath,
}

/// One flagged occurrence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// Path relative to the scanned root.
    pub path: PathBuf,
    /// 1-based line number.
    pub line: usize,
    /// What matched.
    pub kind: HitKind,
    /// The matched text.
    pub matched: String,
}

impl fmt::Display for Hit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let why = match &self.kind {
            HitKind::Denylisted(p) => format!("denylisted pattern `{p}`"),
            HitKind::HomePath => "absolute home path (use `~` or a placeholder)".to_owned(),
        };
        write!(
            f,
            "{}:{}: {why}: {}",
            self.path.display(),
            self.line,
            self.matched
        )
    }
}

/// Scan `text` (the contents of `path`) for hits.
pub fn scan_text(path: &Path, text: &str, denylist: &Denylist) -> Vec<Hit> {
    let mut hits = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let mut push = |kind, matched: &str| {
            hits.push(Hit {
                path: path.to_path_buf(),
                line: i + 1,
                kind,
                matched: matched.to_owned(),
            })
        };
        for re in &denylist.patterns {
            for m in re.find_iter(line) {
                push(HitKind::Denylisted(re.as_str().to_owned()), m.as_str());
            }
        }
        for caps in HOME_PATH.captures_iter(line) {
            let user = caps
                .get(2)
                .or_else(|| caps.get(3))
                .map_or("", |m| m.as_str());
            if !PLACEHOLDER_USERS
                .iter()
                .any(|p| p.eq_ignore_ascii_case(user))
            {
                push(HitKind::HomePath, &caps[1]);
            }
        }
    }
    hits
}

/// Walk `root` and scan every text file, honoring `.gitignore` / `.git/info/exclude`
/// (so build output is skipped) but not the user's global gitignore, so local and CI
/// runs see the same files. Hidden files are scanned; `.git` and the committed
/// denylist itself are not. Non-UTF-8 files are skipped as binary.
pub fn scan_tree(root: &Path, denylist: &Denylist) -> io::Result<Vec<Hit>> {
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_global(false)
        .require_git(false)
        .sort_by_file_name(|a, b| a.cmp(b))
        .filter_entry(|e| e.file_name() != ".git")
        .build();
    let mut hits = Vec::new();
    for entry in walker {
        let entry = entry.map_err(io::Error::other)?;
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let rel = entry.path().strip_prefix(root).unwrap_or(entry.path());
        if rel == Path::new(DENYLIST_FILE) {
            continue;
        }
        let Ok(text) = String::from_utf8(fs::read(entry.path())?) else {
            continue;
        };
        hits.extend(scan_text(rel, &text, denylist));
    }
    Ok(hits)
}

/// Scan every blob and commit message reachable from any ref of the git repository at
/// `root` except the stash (which is never pushed). This function fetches nothing: to
/// cover a forge's pull-request refs, the caller must first fetch them into a local
/// namespace (`git fetch origin '+refs/pull/*:refs/audit/pull/*'`). A hit's path is `<path>@<blob>` for file contents and
/// `commit <sha>` for a message. Old versions of the committed denylist are skipped like
/// the current one.
pub fn scan_history(root: &Path, denylist: &Denylist) -> io::Result<Vec<Hit>> {
    let refs = ["--exclude=refs/stash", "--all"];
    let objects = git(
        root,
        &[&["rev-list", "--objects"][..], &refs].concat(),
        None,
    )?;
    let mut blobs: Vec<(String, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in String::from_utf8_lossy(&objects).lines() {
        let Some((id, path)) = line.split_once(' ') else {
            continue;
        };
        if path != DENYLIST_FILE && seen.insert(id.to_owned()) {
            blobs.push((id.to_owned(), path.to_owned()));
        }
    }
    // Trees and directories are in the list too; `--batch-check` filters them out.
    let input: String = blobs.iter().map(|(id, _)| format!("{id}\n")).collect();
    let types = git(
        root,
        &["cat-file", "--batch-check=%(objecttype)"],
        Some(input.into_bytes()),
    )?;
    let types: Vec<&str> = std::str::from_utf8(&types)
        .map_err(io::Error::other)?
        .lines()
        .collect();
    blobs = blobs
        .into_iter()
        .zip(types)
        .filter(|(_, t)| *t == "blob")
        .map(|(b, _)| b)
        .collect();

    let mut hits = Vec::new();
    for (id, path) in &blobs {
        let bytes = git(root, &["cat-file", "blob", id], None)?;
        if let Ok(text) = String::from_utf8(bytes) {
            let label = PathBuf::from(format!("{path}@{}", &id[..12]));
            hits.extend(scan_text(&label, &text, denylist));
        }
    }
    let log = git(
        root,
        &[&["log", "--format=%H%n%B%x00"][..], &refs].concat(),
        None,
    )?;
    for entry in String::from_utf8_lossy(&log).split('\0') {
        let entry = entry.trim_start_matches('\n');
        if let Some((sha, message)) = entry.split_once('\n') {
            let label = PathBuf::from(format!("commit {}", &sha[..sha.len().min(12)]));
            hits.extend(scan_text(&label, message, denylist));
        }
    }
    Ok(hits)
}

/// Run `git -C root <args>`, feeding `stdin` if given, and return its stdout.
fn git(root: &Path, args: &[&str], stdin: Option<Vec<u8>>) -> io::Result<Vec<u8>> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // Write from another thread so a large stdout cannot deadlock against our input.
    let writer = stdin.map(|input| {
        let mut pipe = child.stdin.take().expect("stdin is piped");
        std::thread::spawn(move || pipe.write_all(&input))
    });
    let out = child.wait_with_output()?;
    if let Some(w) = writer {
        w.join()
            .map_err(|_| io::Error::other("git stdin writer panicked"))??;
    }
    if !out.status.success() {
        return Err(io::Error::other(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(out.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Home paths are assembled at runtime so this file never trips the repo-wide scan.
    fn home(prefix: &str, user: &str) -> String {
        format!("/{prefix}/{user}")
    }

    fn scan(text: &str, list: &Denylist) -> Vec<Hit> {
        scan_text(Path::new("f.md"), text, list)
    }

    #[test]
    fn denylisted_word_is_caught_with_line_number() {
        let list = Denylist::parse("# org names\n(?i)\\bfrobnicorp\\b\n", "test").unwrap();
        let hits = scan("fine\nWe ship Frobnicorp things\n", &list);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].line, 2);
        assert_eq!(hits[0].matched, "Frobnicorp");
        assert_eq!(
            hits[0].kind,
            HitKind::Denylisted("(?i)\\bfrobnicorp\\b".into())
        );
    }

    #[test]
    fn denylist_ignores_comments_and_blanks() {
        let list = Denylist::parse("\n  # comment\n\nfoo\n", "test").unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn invalid_pattern_names_origin_and_line() {
        let err = Denylist::parse("ok\n(unclosed\n", "extra").unwrap_err();
        assert!(
            err.starts_with("extra:2: invalid regex `(unclosed`"),
            "{err}"
        );
    }

    #[test]
    fn home_paths_are_caught() {
        let list = Denylist::default();
        for path in [
            home("Users", "alice"),
            home("home", "bob"),
            format!("{}/code/x", home("Users", "carol.d")),
            format!("cd {}", home("home", "dave")),
            format!("\"{}\"", home("Users", "erin")),
            format!("file://{}", home("Users", "frank")),
            ["C:", "Users", "gina"].join("\\"),
        ] {
            let hits = scan(&path, &list);
            assert_eq!(hits.len(), 1, "expected a hit in {path:?}");
            assert_eq!(hits[0].kind, HitKind::HomePath);
        }
    }

    #[test]
    fn placeholders_and_non_home_paths_are_allowed() {
        let list = Denylist::default();
        for text in [
            "~/code/side-tool",
            "~/work/acme/billing-api",
            &home("Users", "<name>"),
            &home("home", "user"),
            &home("Users", "Shared"),
            &format!("{}/.linuxbrew/bin", home("home", "linuxbrew")),
            "/Volumes/work/acme",
            "src/home/mod.rs",
            "https://example.com/home/page",
            "./home/x",
        ] {
            assert!(scan(text, &list).is_empty(), "unexpected hit in {text:?}");
        }
    }

    #[test]
    fn history_scan_finds_a_leak_removed_in_a_later_commit() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let run = |args: &[&str]| {
            let id = ["-c", "user.name=t", "-c", "user.email=t@example.com"];
            git(root, &[&id[..], args].concat(), None).unwrap();
        };
        run(&["init", "-q"]);
        fs::write(root.join("notes.md"), "ok\nwe ship Frobnicorp things\n").unwrap();
        fs::write(root.join(DENYLIST_FILE), "frobnicorp\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "add notes"]);
        fs::write(root.join("notes.md"), "ok\n").unwrap();
        run(&["commit", "-q", "-am", "scrub Frobnicorp"]);

        let list = Denylist::parse("(?i)frobnicorp", "test").unwrap();
        assert!(scan_tree(root, &list).unwrap().is_empty());
        let hits = scan_history(root, &list).unwrap();
        let where_: Vec<String> = hits
            .iter()
            .map(|h| format!("{}:{}", h.path.display(), h.line))
            .collect();
        assert_eq!(hits.len(), 2, "{where_:?}");
        // `scan_history` scans every blob before any commit message, so the blob hit is first.
        assert!(where_[0].starts_with("notes.md@") && where_[0].ends_with(":2"));
        assert!(where_[1].starts_with("commit ") && where_[1].ends_with(":1"));
    }

    #[test]
    fn load_merges_committed_file_and_env_sources() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(DENYLIST_FILE), "alpha\n").unwrap();
        let extra = dir.path().join("private.txt");
        fs::write(&extra, "gamma\n").unwrap();
        let extra_path = extra.to_str().unwrap().to_owned();
        let list = Denylist::load(dir.path(), |k| match k {
            ENV_PATTERNS => Some("beta\n# c\ndelta".into()),
            ENV_FILE => Some(extra_path.clone()),
            _ => None,
        })
        .unwrap();
        assert_eq!(list.len(), 4);
        let hits = scan("alpha beta gamma delta", &list);
        assert_eq!(hits.len(), 4);
    }

    #[test]
    fn scan_tree_respects_gitignore_and_skips_git_and_denylist() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(DENYLIST_FILE), "secretco\n").unwrap();
        fs::write(root.join(".gitignore"), "target/\n").unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        fs::write(root.join("target/out.txt"), "secretco").unwrap();
        fs::write(root.join(".git"), format!("gitdir: {}", home("Users", "x"))).unwrap();
        fs::create_dir_all(root.join(".github")).unwrap();
        fs::write(root.join(".github/ci.yml"), "name: secretco\n").unwrap();
        fs::write(root.join("bin.dat"), [0xff, 0xfe, 0x00]).unwrap();
        fs::write(
            root.join("notes.md"),
            format!("ok\nsee {}\n", home("home", "zed")),
        )
        .unwrap();

        let list = Denylist::load(root, |_| None).unwrap();
        let hits = scan_tree(root, &list).unwrap();
        let summary: Vec<String> = hits
            .iter()
            .map(|h| format!("{}:{}", h.path.display(), h.line))
            .collect();
        assert_eq!(summary, [".github/ci.yml:1", "notes.md:2"]);
    }
}
