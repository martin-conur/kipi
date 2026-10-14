//! Keeping a store in step with its own git remote: `kipi pull`, `kipi push` and
//! `kipi init --clone` (roadmap §11). Every git call goes through [`crate::git`], one
//! store at a time, in that store's directory; nothing ever moves between stores.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use toml_edit::DocumentMut;

use crate::config::{BINDINGS_FILE, Env};
use crate::doctor::{Finding, Level};
use crate::error::{Error, Result};
use crate::git::{self, Tracking};
use crate::model::{Store, StoreSet};

/// What `pull` did to one store.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pulled {
    /// The store is not in a git work tree; skipped.
    NotGit,
    /// Its branch has no upstream; skipped.
    NoUpstream,
    /// Nothing new upstream (commits of its own may still be unpushed).
    UpToDate,
    /// Fast-forwarded by this many commits.
    FastForwarded {
        /// Commits pulled.
        commits: u32,
    },
    /// Both sides have commits; nothing was changed.
    Diverged(Tracking),
}

/// Fetch the store's upstream and fast-forward to it. Diverged histories are reported,
/// never merged; the working tree is left untouched then. A failed fetch, or a
/// fast-forward git refuses (an uncommitted change in the way), is an error.
pub fn pull(root: &Path) -> Result<Pulled> {
    let Some(status) = git::status(root) else {
        return Ok(Pulled::NotGit);
    };
    if status.upstream.is_none() {
        return Ok(Pulled::NoUpstream);
    }
    git::fetch(root)?;
    let tracking = git::status(root)
        .and_then(|s| s.upstream)
        .unwrap_or_default();
    match tracking {
        Tracking { behind: 0, .. } => Ok(Pulled::UpToDate),
        Tracking { ahead: 0, behind } => {
            git::fast_forward(root)?;
            Ok(Pulled::FastForwarded { commits: behind })
        }
        diverged => Ok(Pulled::Diverged(diverged)),
    }
}

/// What changed in a store since its last commit, by kind, for the commit message.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// Repo entries (`[repos.<name>]`) and notes (`repos/<name>/`).
    pub repos: BTreeSet<String>,
    /// Skills (`skills/<name>/`).
    pub skills: BTreeSet<String>,
    /// Scopes (`[scopes.<name>]`) and conventions (`conventions/<scope>.md`).
    pub scopes: BTreeSet<String>,
    /// Any other changed file, relative to the store.
    pub files: BTreeSet<String>,
}

impl Changes {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.repos.is_empty()
            && self.skills.is_empty()
            && self.scopes.is_empty()
            && self.files.is_empty()
    }
}

/// The uncommitted changes of the store at `root` (`generated/` left out).
pub fn changes(root: &Path) -> Result<Changes> {
    let mut out = Changes::default();
    for path in git::changed_paths(root)? {
        let mut parts = path.splitn(3, '/');
        match (parts.next(), parts.next(), parts.next()) {
            (Some(BINDINGS_FILE), None, None) => {
                let before = git::head_file(root, BINDINGS_FILE).unwrap_or_default();
                let after = std::fs::read_to_string(root.join(BINDINGS_FILE)).unwrap_or_default();
                bindings_changes(&before, &after, &mut out);
            }
            (Some("skills"), Some(name), Some(_)) => {
                out.skills.insert(name.to_owned());
            }
            (Some("repos"), Some(name), Some(_)) => {
                out.repos.insert(name.to_owned());
            }
            (Some("conventions"), Some(file), None) if file.ends_with(".md") => {
                out.scopes.insert(file.trim_end_matches(".md").to_owned());
            }
            _ => {
                out.files.insert(path);
            }
        }
    }
    Ok(out)
}

/// The `[repos.*]` and `[scopes.*]` tables that differ between two `bindings.toml`
/// texts; any other difference counts as a change to the file itself.
fn bindings_changes(before: &str, after: &str, out: &mut Changes) {
    let (Ok(old), Ok(new)) = (before.parse::<DocumentMut>(), after.parse::<DocumentMut>()) else {
        out.files.insert(BINDINGS_FILE.to_owned());
        return;
    };
    let mut named = false;
    for (key, set) in [("repos", &mut out.repos), ("scopes", &mut out.scopes)] {
        let table = |doc: &DocumentMut| {
            doc.get(key)
                .and_then(|t| t.as_table_like())
                .map(|t| {
                    t.iter()
                        .map(|(k, v)| (k.to_owned(), v.to_string()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        };
        let (a, b) = (table(&old), table(&new));
        let names: BTreeSet<&String> = a.iter().chain(&b).map(|(k, _)| k).collect();
        for name in names {
            let find =
                |t: &[(String, String)]| t.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());
            if find(&a) != find(&b) {
                set.insert(name.clone());
                named = true;
            }
        }
    }
    let rest = |doc: &DocumentMut| {
        doc.iter()
            .filter(|(k, _)| *k != "repos" && *k != "scopes")
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
    };
    if rest(&old) != rest(&new) || !named {
        out.files.insert(BINDINGS_FILE.to_owned());
    }
}

/// The commit message `push` writes for `changes` in store `store`: a subject naming the
/// store and what changed, then one line per kind.
pub fn message(store: &str, changes: &Changes) -> String {
    let kinds = [
        ("repos", &changes.repos),
        ("skills", &changes.skills),
        ("scopes", &changes.scopes),
        ("files", &changes.files),
    ];
    let listed: Vec<(&str, Vec<&str>)> = kinds
        .iter()
        .filter(|(_, set)| !set.is_empty())
        .map(|(kind, set)| (*kind, set.iter().map(String::as_str).collect()))
        .collect();
    let summary: Vec<String> = listed
        .iter()
        .map(|(kind, names)| {
            let shown: Vec<&str> = names.iter().take(3).copied().collect();
            let more = if names.len() > 3 { ", …" } else { "" };
            format!("{kind} {}{more}", shown.join(", "))
        })
        .collect();
    let mut text = format!("kipi: update store {store}");
    if !summary.is_empty() {
        text.push_str(": ");
        text.push_str(&summary.join("; "));
    }
    text.push('\n');
    if !listed.is_empty() {
        text.push('\n');
        for (kind, names) in &listed {
            text.push_str(&format!("{kind}: {}\n", names.join(", ")));
        }
    }
    text
}

/// What `push` did to one store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pushed {
    /// Nothing to commit and nothing unpushed.
    UpToDate,
    /// Pushed this many commits, `committed` holding the message of the one `push` made.
    Pushed {
        /// Commits sent.
        commits: u32,
        /// The message of the commit made first, if there was anything to commit.
        committed: Option<String>,
    },
}

/// Why a store cannot be pushed; checked before anything is committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The store is not in a git work tree.
    NotGit,
    /// Its branch has no upstream; `branch` names it when HEAD is on one.
    NoUpstream {
        /// The current branch.
        branch: Option<String>,
    },
    /// Machine-only state lies inside the store.
    MachineState(PathBuf),
    /// Doctor found error-level problems in the store.
    Findings(Vec<Finding>),
}

/// Whether the store at `root` may be pushed: a git work tree with an upstream, holding
/// no machine-only state, with no error-level doctor finding in `findings` attributed to
/// it ([`blocking`]).
pub fn check_push(
    store: &Store,
    stores: &StoreSet,
    findings: &[Finding],
    env: &Env,
) -> Option<Refusal> {
    let root = &store.root;
    let Some(status) = git::status(root) else {
        return Some(Refusal::NotGit);
    };
    if status.upstream.is_none() {
        return Some(Refusal::NoUpstream {
            branch: git::branch(root),
        });
    }
    if let Some(path) = machine_state_inside(root, env) {
        return Some(Refusal::MachineState(path));
    }
    let errors = blocking(store, stores, findings);
    (!errors.is_empty()).then_some(Refusal::Findings(errors))
}

/// Commit every change in the store at `root` (`message` overriding the generated one),
/// then push. Run only after [`check_push`] passed.
pub fn push(root: &Path, store: &str, message_override: Option<&str>) -> Result<Pushed> {
    let changes = changes(root)?;
    let committed = if changes.is_empty() {
        None
    } else {
        let text = match message_override {
            Some(m) => m.to_owned(),
            None => message(store, &changes),
        };
        git::commit_dir(root, &text)?;
        Some(text)
    };
    let ahead = git::status(root)
        .and_then(|s| s.upstream)
        .map_or(0, |t| t.ahead);
    if ahead == 0 {
        return Ok(Pushed::UpToDate);
    }
    git::push(root)?;
    Ok(Pushed::Pushed {
        commits: ahead,
        committed,
    })
}

/// The error-level findings about store `store`: located in one of its files, or about
/// one of its repo entries.
pub fn blocking(store: &Store, stores: &StoreSet, findings: &[Finding]) -> Vec<Finding> {
    findings
        .iter()
        .filter(|f| f.level == Level::Error)
        .filter(|f| {
            let in_file = f.at.as_ref().is_some_and(|at| {
                at.store.as_deref() == Some(store.name.as_str()) || at.file.starts_with(&store.root)
            });
            let of_entry = f
                .repo
                .as_deref()
                .and_then(|r| stores.repo(r))
                .is_some_and(|(s, _)| s.name == store.name);
            in_file || of_entry
        })
        .cloned()
        .collect()
}

/// `config.toml`, `machine.toml` or the cache directory, when one lies inside `root`.
pub fn machine_state_inside(root: &Path, env: &Env) -> Option<PathBuf> {
    let real = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let root = real(root);
    [env.config_file(), env.machine_file(), env.cache_dir.clone()]
        .into_iter()
        .find(|p| p.exists() && real(p).starts_with(&root))
}

/// Clone `url` into `dest` for `init --clone`: `dest` must not exist or be an empty
/// directory, and the clone must hold a `bindings.toml`. A clone that is not a store is
/// left on disk, named in the error.
pub fn clone_store(url: &str, dest: &Path) -> Result<()> {
    if dest.is_file() {
        return Err(Error::usage(
            format!("{} is a file, not a directory", dest.display()),
            "pass a new or empty directory for --store",
        ));
    }
    let non_empty = std::fs::read_dir(dest).is_ok_and(|mut d| d.next().is_some());
    if non_empty {
        return Err(Error::usage(
            format!(
                "{} is not empty; --clone needs a new or empty directory",
                dest.display()
            ),
            format!(
                "pick another --store path, or drop --clone to mount the store already at {}",
                dest.display()
            ),
        ));
    }
    git::clone_full(url, dest)?;
    if !dest.join(BINDINGS_FILE).is_file() {
        return Err(Error::usage(
            format!(
                "the clone of {url} at {} has no {BINDINGS_FILE}: it is not a kipi store",
                dest.display()
            ),
            format!(
                "check the URL; the clone was kept, so delete {} before trying again",
                dest.display()
            ),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bindings_changes_name_the_tables_that_differ() {
        let before = "[store]\nname = \"p\"\n\n[scopes.a]\nroots = []\n\n[repos.x]\nsummary = \"x\"\n\n[repos.y]\nsummary = \"y\"\n";
        let after = "[store]\nname = \"p\"\n\n[scopes.a]\nroots = [\"~/c\"]\n\n[repos.x]\nsummary = \"x\"\n\n[repos.z]\nsummary = \"z\"\n";
        let mut c = Changes::default();
        bindings_changes(before, after, &mut c);
        assert_eq!(c.repos, BTreeSet::from(["y".to_owned(), "z".to_owned()]));
        assert_eq!(c.scopes, BTreeSet::from(["a".to_owned()]));
        assert!(c.files.is_empty(), "{c:?}");

        let mut c = Changes::default();
        bindings_changes(before, &before.replace("\"p\"", "\"q\""), &mut c);
        assert_eq!(c.files, BTreeSet::from([BINDINGS_FILE.to_owned()]));
    }

    #[test]
    fn message_lists_every_kind_and_trims_the_subject() {
        let c = Changes {
            repos: ["a", "b", "c", "d"].map(str::to_owned).into(),
            skills: ["review".to_owned()].into(),
            ..Changes::default()
        };
        assert_eq!(
            message("personal", &c),
            "kipi: update store personal: repos a, b, c, …; skills review\n\n\
             repos: a, b, c, d\nskills: review\n"
        );
    }
}
