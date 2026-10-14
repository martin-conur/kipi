//! `kipi pull`, `kipi push` and `kipi init --clone` (roadmap §7, §11): each store syncs
//! on its own, with its own git remote. A store that fails stops only itself; the
//! command reports every store, then exits 1 (2 for a push refused on doctor errors).

use std::path::{Path, PathBuf};

use kipi_core::authoring::{self, Change, InitOptions, Op, Outcome};
use kipi_core::config::{self, Env};
use kipi_core::doctor::{self, Checkouts, Finding};
use kipi_core::error::closest;
use kipi_core::git;
use kipi_core::model::{Config, Store};
use kipi_core::store_git::{self, Pulled, Pushed, Refusal};

use super::{Loaded, sync};
use crate::error::Failure;
use crate::output::color::styles;
use crate::output::{Output, json};

/// What happened to one store.
pub struct Row {
    /// `[store] name`.
    pub name: String,
    /// Its directory.
    pub path: PathBuf,
    /// The stable result word (`docs/json.md`, "Pull and push").
    pub result: &'static str,
    /// Commits pulled or pushed (or that would be).
    pub commits: u32,
    /// The message of the commit `push` made (or would make).
    pub commit_message: Option<String>,
    /// Why the store was skipped, refused or failed.
    pub reason: Option<String>,
    /// What to do about it.
    pub hint: Option<String>,
    /// The doctor errors that refused a push.
    pub findings: Vec<Finding>,
    /// Whether the store stops the command from succeeding.
    pub failed: bool,
}

impl Row {
    fn new(store: &Store, result: &'static str) -> Self {
        Row {
            name: store.name.clone(),
            path: store.root.clone(),
            result,
            commits: 0,
            commit_message: None,
            reason: None,
            hint: None,
            findings: Vec::new(),
            failed: false,
        }
    }

    fn because(mut self, reason: impl Into<String>, hint: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self.hint = Some(hint.into());
        self
    }

    fn failing(mut self) -> Self {
        self.failed = true;
        self
    }
}

/// Each configured store, loaded on its own (a set that no longer validates must still
/// be pullable), and the ones `names` selects (all when empty).
fn selected(
    env: &Env,
    cfg: &Config,
    store_flags: &[PathBuf],
    names: &[String],
) -> Result<(Vec<Store>, Vec<usize>), Failure> {
    config::check_config(store_flags, env, cfg)?;
    let stores = config::store_paths(store_flags, env, cfg)
        .iter()
        .map(|p| config::load_store(p, env))
        .collect::<Result<Vec<Store>, _>>()?;
    if names.is_empty() {
        let all = (0..stores.len()).collect();
        return Ok((stores, all));
    }
    let mut picked = Vec::new();
    for name in names {
        let Some(i) = stores.iter().position(|s| &s.name == name) else {
            let known = stores.iter().map(|s| s.name.as_str());
            let hint = match closest(name, known.clone()) {
                Some(near) => format!("did you mean `{near}`?"),
                None => format!("mounted stores: {}", known.collect::<Vec<_>>().join(", ")),
            };
            return Err(Failure::user(format!("no mounted store `{name}`"), hint));
        };
        if !picked.contains(&i) {
            picked.push(i);
        }
    }
    Ok((stores, picked))
}

/// `kipi pull`.
pub fn pull(
    env: &Env,
    store_flags: &[PathBuf],
    names: &[String],
    no_sync: bool,
    dry_run: bool,
    out: &Output,
) -> Result<(), Failure> {
    let cfg = config::load_config(env)?;
    let (stores, picked) = selected(env, &cfg, store_flags, names)?;
    let rows: Vec<Row> = picked
        .iter()
        .map(|&i| pull_one(&stores[i], dry_run, out))
        .collect();
    if !out.json {
        // Reported before the sync, which fails outright when a pulled store no longer
        // validates.
        out.print(&report("pull", &rows, dry_run, out))?;
    }
    let synced = if no_sync || dry_run {
        None
    } else {
        // Re-validate every mounted store as a set, as any command would, then sync.
        Some(Loaded::load(env, store_flags, out).and_then(|loaded| {
            let args = sync::Args {
                repos: &[],
                all: true,
                agents: &[],
                force: false,
                allow_tracked: false,
                dry_run: false,
                hook: false,
            };
            sync::execute(env, &loaded, &args, out)
        }))
    };
    if out.json {
        // One document whatever happened: a failed sync still carries every store's row.
        let parts = match &synced {
            Some(Err(e)) => Some(e.parts()),
            _ => None,
        };
        let sync = match (&synced, &parts) {
            (Some(Ok(Some(outcome))), _) => Some(Ok(outcome)),
            (Some(Err(_)), Some((error, hint))) => Some(Err((error.as_str(), hint.as_str()))),
            _ => None,
        };
        let text = json::pull(&rows, sync, dry_run).map_err(|e| Failure::json("the pull", e))?;
        out.print(&text)?;
    }
    match synced {
        Some(Err(e)) => return Err(e),
        Some(Ok(Some(outcome))) if !out.json => {
            out.print(&format!("\n{}", sync::human(&outcome, false, out)))?;
        }
        _ => {}
    }
    finish(&rows, "pull")
}

fn pull_one(store: &Store, dry_run: bool, out: &Output) -> Row {
    let root = &store.root;
    let skipped =
        |result, reason: &str, hint: String| Row::new(store, result).because(reason, hint);
    let not_git = || {
        skipped(
            "not_git",
            "not a git repo",
            "sync this store your own way; `git init` it and add a remote to use `kipi pull`"
                .to_owned(),
        )
    };
    let no_upstream = || {
        skipped(
            "no_upstream",
            "its branch has no upstream",
            format!(
                "set one with `git -C {} branch --set-upstream-to origin/<branch>`",
                out.path(root)
            ),
        )
    };
    if dry_run {
        // A dry run fetches nothing: it reports against the last fetch.
        return match git::status(root) {
            None => not_git(),
            Some(s) => match s.upstream {
                None => no_upstream(),
                Some(t) if t.behind == 0 => Row::new(store, "up_to_date"),
                Some(t) if t.ahead > 0 => diverged(store, t, out),
                Some(t) => Row {
                    commits: t.behind,
                    ..Row::new(store, "would_fast_forward")
                },
            },
        };
    }
    match store_git::pull(root) {
        Ok(Pulled::NotGit) => not_git(),
        Ok(Pulled::NoUpstream) => no_upstream(),
        Ok(Pulled::UpToDate) => Row::new(store, "up_to_date"),
        Ok(Pulled::FastForwarded { commits }) => Row {
            commits,
            ..Row::new(store, "fast_forwarded")
        },
        Ok(Pulled::Diverged(t)) => diverged(store, t, out),
        Err(e) => Row::new(store, "failed")
            .because(e.to_string(), e.hint())
            .failing(),
    }
}

fn diverged(store: &Store, t: git::Tracking, out: &Output) -> Row {
    Row::new(store, "diverged")
        .because(
            format!(
                "diverged from its upstream ({} ahead, {} behind); nothing was changed",
                t.ahead, t.behind
            ),
            format!(
                "cd {} && git pull --rebase, then run `kipi pull` again",
                out.path(&store.root)
            ),
        )
        .failing()
}

/// `kipi push`.
pub fn push(
    env: &Env,
    store_flags: &[PathBuf],
    names: &[String],
    message: Option<&str>,
    dry_run: bool,
    out: &Output,
) -> Result<(), Failure> {
    let cfg = config::load_config(env)?;
    let (stores, picked) = selected(env, &cfg, store_flags, names)?;
    let machine = config::load_machine(env)?;
    let caches = super::caches(env, &cfg);
    let diagnosis = doctor::diagnose(
        stores.clone(),
        &machine,
        &Checkouts::read(&machine),
        &env.home,
        None,
        &caches,
    )?;
    let explicit = !names.is_empty();
    let rows: Vec<Row> = picked
        .iter()
        .map(|&i| {
            let store = &stores[i];
            let refusal = store_git::check_push(store, &diagnosis.stores, &diagnosis.findings, env);
            match refusal {
                Some(r) => refused(store, r, explicit, out),
                None if dry_run => would_push(store, message),
                None => match store_git::push(&store.root, &store.name, message) {
                    Ok(Pushed::UpToDate) => Row::new(store, "up_to_date"),
                    Ok(Pushed::Pushed { commits, committed }) => Row {
                        commits,
                        commit_message: committed,
                        ..Row::new(store, "pushed")
                    },
                    Err(e) => Row::new(store, "failed")
                        .because(e.to_string(), e.hint())
                        .failing(),
                },
            }
        })
        .collect();
    if out.json {
        let text = json::push(&diagnosis.stores, &rows, dry_run)
            .map_err(|e| Failure::json("the push", e))?;
        out.print(&text)?;
    } else {
        out.print(&report("push", &rows, dry_run, out))?;
    }
    let findings: usize = rows.iter().map(|r| r.findings.len()).sum();
    if findings > 0 {
        return Err(Failure::Findings(findings));
    }
    finish(&rows, "push")
}

/// The row of a store `push` will not touch. Without a remote to push to, a store that
/// was not named is skipped quietly: a local-only store is normal.
fn refused(store: &Store, refusal: Refusal, explicit: bool, out: &Output) -> Row {
    let root = out.path(&store.root);
    match refusal {
        Refusal::NotGit if !explicit => Row::new(store, "not_git").because(
            "not a git repo",
            "nothing to push; sync this store your own way",
        ),
        Refusal::NotGit => Row::new(store, "refused")
            .because(
                "not a git repo",
                format!("`git init` {root} and add a remote to push it"),
            )
            .failing(),
        Refusal::NoUpstream { branch } => {
            let branch = branch.unwrap_or_else(|| "<branch>".to_owned());
            let hint =
                format!("push once by hand to set it: `git -C {root} push -u origin {branch}`");
            let row = Row::new(store, if explicit { "refused" } else { "no_upstream" })
                .because("its branch has no upstream; nothing was committed", hint);
            if explicit { row.failing() } else { row }
        }
        Refusal::MachineState(path) => {
            let path = out.path(&path);
            Row::new(store, "refused")
                .because(
                    format!(
                        "{path} is machine-only state and lies inside the store; nothing was committed"
                    ),
                    format!(
                        "move {path} out of the store: point KIPI_CONFIG_DIR / KIPI_CACHE_DIR elsewhere, or move the store"
                    ),
                )
                .failing()
        }
        Refusal::Findings(findings) => Row {
            findings,
            ..Row::new(store, "refused")
                .because(
                    "doctor found error-level problems in this store; nothing was committed",
                    "fix each as its hint says (`kipi doctor`), then push again",
                )
                .failing()
        },
    }
}

/// What `push --dry-run` reports: the commit it would make and the commits it would send.
fn would_push(store: &Store, message: Option<&str>) -> Row {
    let changes = match store_git::changes(&store.root) {
        Ok(c) => c,
        Err(e) => {
            return Row::new(store, "failed")
                .because(e.to_string(), e.hint())
                .failing();
        }
    };
    let ahead = git::status(&store.root)
        .and_then(|s| s.upstream)
        .map_or(0, |t| t.ahead);
    let commit = (!changes.is_empty())
        .then(|| message.map_or_else(|| store_git::message(&store.name, &changes), str::to_owned));
    let commits = ahead + u32::from(commit.is_some());
    if commits == 0 {
        return Row::new(store, "up_to_date");
    }
    Row {
        commits,
        commit_message: commit,
        ..Row::new(store, "would_push")
    }
}

/// The per-store report of `pull` or `push`.
fn report(command: &str, rows: &[Row], dry_run: bool, out: &Output) -> String {
    let p = &out.paint;
    let mut text = String::new();
    if rows.is_empty() {
        text.push_str("No store to ");
        text.push_str(command);
        text.push_str(".\n");
        return text;
    }
    let width = rows.iter().map(|r| r.name.len()).max().unwrap_or(0);
    let commits = |n: u32| format!("{n} commit{}", if n == 1 { "" } else { "s" });
    for r in rows {
        let what = match r.result {
            "up_to_date" => "up to date".to_owned(),
            "fast_forwarded" => format!("pulled {}", commits(r.commits)),
            "would_fast_forward" => format!("would pull {}", commits(r.commits)),
            "pushed" => format!("pushed {}", commits(r.commits)),
            "would_push" => format!("would push {}", commits(r.commits)),
            "not_git" | "no_upstream" => {
                format!("skipped: {}", r.reason.as_deref().unwrap_or_default())
            }
            _ => r.reason.clone().unwrap_or_default(),
        };
        let name = format!("{:<width$}", r.name);
        text.push_str(&format!("{}  {what}\n", p.paint(name, styles::name())));
        let indent = " ".repeat(width + 2);
        if let Some(message) = &r.commit_message {
            let verb = if dry_run { "would commit" } else { "committed" };
            let subject = message.lines().next().unwrap_or_default();
            text.push_str(&format!(
                "{indent}{}\n",
                p.dim(format!("{verb}: {subject}"))
            ));
        }
        for f in &r.findings {
            text.push_str(&format!(
                "{indent}{} {}: {}\n",
                p.error(f.level.as_str()),
                f.code,
                f.message
            ));
            text.push_str(&format!("{indent}  {} {}\n", p.hint("hint:"), f.hint));
        }
        if r.failed
            && let Some(hint) = &r.hint
        {
            text.push_str(&format!("{indent}{} {hint}\n", p.hint("hint:")));
        }
    }
    text
}

/// A failure naming the stores that stopped, after every store was reported.
fn finish(rows: &[Row], command: &str) -> Result<(), Failure> {
    let failed: Vec<&Row> = rows.iter().filter(|r| r.failed).collect();
    match failed.as_slice() {
        [] => Ok(()),
        [one] => Err(Failure::user(
            format!(
                "{command} stopped for store `{}`: {}",
                one.name,
                one.reason.as_deref().unwrap_or_default()
            ),
            one.hint.clone().unwrap_or_default(),
        )),
        many => Err(Failure::user(
            format!(
                "{command} stopped for {} stores: {}",
                many.len(),
                many.iter()
                    .map(|r| format!("`{}`", r.name))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            "each store's line above says why and what to do",
        )),
    }
}

/// `kipi init --clone <url>`: clone into the --store directory, then mount it as
/// `kipi init` mounts an existing store. With `--dry-run` nothing is cloned, so nothing
/// can be validated yet.
pub fn init_clone(
    env: &Env,
    opts: &InitOptions,
    url: &str,
    dry_run: bool,
    out: &Output,
) -> Result<(), Failure> {
    let root = authoring::store_root(env, opts.path)?;
    let clone = Change::new(
        Op::Add,
        "git clone",
        format!("{url} into {}", root.display()),
    );
    if dry_run {
        let mut outcome = Outcome::default();
        outcome
            .changes
            .push(clone.note("cloned, validated and mounted when run without --dry-run"));
        return super::authoring::finish("init", "init".to_owned(), &outcome, env, true, out);
    }
    store_git::clone_store(url, &root)?;
    let cfg = config::load_config(env)?;
    let machine = config::load_machine(env)?;
    let opts = InitOptions {
        path: Some(&root),
        ..*opts
    };
    let mut outcome = authoring::init(env, &cfg, &machine, &opts).map_err(|e| kept(e, &root))?;
    outcome.changes.insert(0, clone);
    super::authoring::finish("init", "init".to_owned(), &outcome, env, false, out)?;
    if !out.json {
        out.print("\nNext: `kipi scan`, then `kipi sync --all`.\n")?;
    }
    Ok(())
}

/// `e`, its hint saying the clone at `root` was kept.
fn kept(e: kipi_core::error::Error, root: &Path) -> Failure {
    Failure::user(
        e.to_string(),
        format!(
            "{}; the clone at {} was kept and is not mounted",
            e.hint(),
            root.display()
        ),
    )
}
