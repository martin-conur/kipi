//! `kipi sync [repo...] [--all]`: plan → diff → apply, then a per-repo summary of links,
//! blocks and grants added or removed, always-on size vs budget, and warnings (roadmap
//! §7, §8). `--dry-run` lists every change, with JSON diffs for the read grant.
//!
//! Named repos and `--all` cover each checkout's linked worktrees too; with neither,
//! `kipi sync` syncs the working tree it is run in (a checkout or a linked worktree of
//! one). `--hook` is that, run by the `post-checkout` hook: silent outside a known repo.
//!
//! Each working tree's due sync commands (`on_sync`) run after it is materialized, if
//! approved on this machine: `--allow-commands` approves them, and an interactive run asks
//! once. A failed command is reported per repo and makes the exit code 1.

use std::collections::BTreeSet;
use std::io::{BufRead, IsTerminal, Write};

use kipi_core::apply::{self, Report};
use kipi_core::config::{self, Env};
use kipi_core::error::closest;
use kipi_core::model::AgentId;
use kipi_core::on_sync;
use kipi_core::plan::{CommandPlan, Plan, PlannedCommand, RepoPlan, SkipReason};
use kipi_core::sync::{self, CommandState, Outcome, RepoOutcome, RepoState, Selection, Tally};

use super::Loaded;
use crate::error::Failure;
use crate::output::{Output, diff, json};

/// The command's arguments.
pub struct Args<'a> {
    /// Repo entries to sync.
    pub repos: &'a [String],
    /// `--all`.
    pub all: bool,
    /// `--agents`.
    pub agents: &'a [String],
    /// `--force`.
    pub force: bool,
    /// `--allow-tracked`.
    pub allow_tracked: bool,
    /// `--allow-commands`: approve the selected repos' sync commands on this machine.
    pub allow_commands: bool,
    /// `--dry-run`.
    pub dry_run: bool,
    /// `--hook`: run by the `post-checkout` hook.
    pub hook: bool,
}

/// Sync, then report.
pub fn run(env: &Env, loaded: &Loaded, args: &Args, out: &Output) -> Result<(), Failure> {
    let Some(outcome) = execute(env, loaded, args, out)? else {
        return Ok(());
    };
    if out.json {
        let text = json::sync(&outcome, args.dry_run).map_err(|e| Failure::json("the sync", e))?;
        out.print(&text)?;
    } else {
        out.print(&human(&outcome, args.dry_run, out))?;
    }
    match failed_commands(&outcome) {
        Some(failure) if !args.hook => Err(failure),
        _ => Ok(()),
    }
}

/// The failure to exit with when a sync command failed: exit code 1, after the report.
pub fn failed_commands(outcome: &Outcome) -> Option<Failure> {
    let failed: Vec<String> = outcome
        .repos
        .iter()
        .flat_map(|r| {
            r.commands
                .iter()
                .filter(|c| c.state.failed())
                .map(|c| format!("{}/{}", r.name, c.name))
        })
        .collect();
    (!failed.is_empty()).then(|| {
        Failure::user(
            format!(
                "{} sync command{} failed: {}",
                failed.len(),
                if failed.len() == 1 { "" } else { "s" },
                failed.join(", ")
            ),
            "see the output above; failed commands rerun on the next `kipi sync`",
        )
    })
}

/// Sync without reporting; `None` when `--hook` finds no repo here.
pub fn execute(
    env: &Env,
    loaded: &Loaded,
    args: &Args,
    out: &Output,
) -> Result<Option<Outcome>, Failure> {
    let selection = if args.all {
        Selection::All
    } else if args.repos.is_empty() {
        let here = std::env::current_dir()
            .ok()
            .and_then(|cwd| sync::locate(&loaded.plan, &cwd));
        match here {
            Some((repo, dir)) => Selection::Tree {
                repo: repo.name.clone(),
                dir,
            },
            None if args.hook => return Ok(None),
            None => return Err(no_repo_here(loaded)),
        }
    } else {
        for name in args.repos {
            loaded.repo(name)?;
        }
        Selection::Only(args.repos.to_vec())
    };
    let opts = sync::Options {
        agents: parse_agents(args.agents)?,
        apply: apply::Options {
            dry_run: args.dry_run,
            force: args.force,
        },
        allow_tracked: args.allow_tracked,
        home: Some(env.home.clone()),
        worktrees: true,
        run_commands: !args.dry_run,
    };
    let _lock = if args.dry_run {
        None
    } else {
        Some(apply::Lock::acquire(&env.config_dir)?)
    };
    let approved = approve(env, loaded, &selection, args, out)?;
    let outcome = sync::run(
        &loaded.stores,
        &loaded.machine,
        approved.as_ref().unwrap_or(&loaded.plan),
        &selection,
        &opts,
        &loaded.caches,
    )?;
    loaded.save_caches(out);
    Ok(Some(outcome))
}

/// Approve the selection's unapproved sync commands, with `--allow-commands` or a yes to
/// the one `[y/N]` an interactive run asks: written to `machine.toml` (not in a dry run)
/// and returned as the plan to sync. `None` when nothing was approved.
fn approve(
    env: &Env,
    loaded: &Loaded,
    selection: &Selection,
    args: &Args,
    out: &Output,
) -> Result<Option<Plan>, Failure> {
    let pending = unapproved(&loaded.plan, selection);
    if pending.is_empty() || args.hook {
        return Ok(None);
    }
    let interactive = !out.json
        && out.verbosity >= 0
        && !args.dry_run
        && std::io::stdin().is_terminal()
        && std::io::stderr().is_terminal();
    if !args.allow_commands && !interactive {
        return Ok(None);
    }
    let p = &out.paint_err;
    let lead = match (args.allow_commands, args.dry_run) {
        (true, true) => "would approve on this machine:",
        (true, false) => "approving on this machine:",
        (false, _) => "these sync commands are not approved on this machine:",
    };
    let mut text = format!("{lead}\n");
    for (repo, cmd) in &pending {
        text.push_str(&format!(
            "  {} {} {}  {}\n",
            p.heading(&cmd.name),
            p.dim(format!("({} · {})", repo.store, cmd.source.rule)),
            p.dim("runs"),
            template(cmd)
        ));
    }
    eprint!("{text}");
    if !args.allow_commands {
        eprint!("approve and run them? [y/N] ");
        let _ = std::io::stderr().flush();
        let mut answer = String::new();
        let _ = std::io::stdin().lock().read_line(&mut answer);
        if !matches!(answer.trim(), "y" | "Y" | "yes" | "Yes") {
            return Ok(None);
        }
    }
    let keys: Vec<String> = pending.iter().map(|(_, c)| c.approval.clone()).collect();
    if !args.dry_run {
        config::approve_commands(env, &loaded.machine, &keys)?;
    }
    let mut plan = loaded.plan.clone();
    plan.approve(&keys.into_iter().collect());
    Ok(Some(plan))
}

/// A command's template as written, shell-quoted for display.
pub fn template(cmd: &PlannedCommand) -> String {
    let words: Vec<String> = cmd.template.iter().map(ToString::to_string).collect();
    on_sync::display(&words)
}

/// The unapproved sync commands `selection` would run, one per approval key, with the
/// first repo that plans each: cloned repos only, and in a linked worktree only those
/// that run in worktrees.
fn unapproved<'p>(
    plan: &'p Plan,
    selection: &Selection,
) -> Vec<(&'p RepoPlan, &'p PlannedCommand)> {
    let repos: Vec<(&RepoPlan, bool)> = match selection {
        Selection::All => plan.repos.iter().map(|r| (r, false)).collect(),
        Selection::Only(names) => names
            .iter()
            .filter_map(|n| plan.repo(n))
            .map(|r| (r, false))
            .collect(),
        Selection::Tree { repo, dir } => plan
            .repo(repo)
            .map(|r| {
                let real = |p: &std::path::Path| p.canonicalize().ok();
                (r, r.path.as_deref().and_then(real) != real(dir))
            })
            .into_iter()
            .collect(),
    };
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for (repo, worktree_only) in repos.into_iter().filter(|(r, _)| r.path.is_some()) {
        for cmd in &repo.on_sync {
            if matches!(cmd.outcome, CommandPlan::Unapproved { .. })
                && (!worktree_only || cmd.worktrees)
                && seen.insert(cmd.approval.as_str())
            {
                out.push((repo, cmd));
            }
        }
    }
    out
}

/// `kipi sync` with nothing named, outside every known repo: say how to name one, with
/// real entries of the mounted stores.
fn no_repo_here(loaded: &Loaded) -> Failure {
    let cloned: Vec<&str> = loaded
        .plan
        .repos
        .iter()
        .filter(|r| r.path.is_some())
        .map(|r| r.name.as_str())
        .collect();
    let hint = match cloned.as_slice() {
        [] => {
            "run it inside a repo kipi knows, or record your checkouts with `kipi scan`".to_owned()
        }
        names => {
            let shown = names
                .iter()
                .take(3)
                .map(|n| format!("`{n}`"))
                .collect::<Vec<_>>();
            let more = if names.len() > 3 { ", …" } else { "" };
            format!(
                "run it inside a repo kipi knows, name entries (`kipi sync <repo>`: {}{more}), or pass `--all`",
                shown.join(", ")
            )
        }
    };
    Failure::user(
        "no repo to sync: the working directory is in no known repo",
        hint,
    )
}

/// `--agents claude,generic` as adapters; `None` when not given.
fn parse_agents(raw: &[String]) -> Result<Option<Vec<AgentId>>, Failure> {
    if raw.is_empty() {
        return Ok(None);
    }
    let names = AgentId::ALL.map(AgentId::as_str);
    let mut agents = Vec::new();
    for name in raw {
        let Some(id) = AgentId::ALL.into_iter().find(|a| a.as_str() == name) else {
            let hint = match closest(name, names) {
                Some(near) => format!("did you mean `{near}`?"),
                None => format!("known agents: {}", names.join(", ")),
            };
            return Err(Failure::user(format!("unknown agent `{name}`"), hint));
        };
        if !agents.contains(&id) {
            agents.push(id);
        }
    }
    Ok(Some(agents))
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// What a tally amounts to, as `+2 links, 1 block written, …`; `None` when nothing.
fn describe(t: &Tally) -> Option<String> {
    let mut parts = Vec::new();
    let mut count = |n: usize, text: String| {
        if n > 0 {
            parts.push(text);
        }
    };
    count(
        t.links_added,
        format!("+{}", plural(t.links_added, "link", "links")),
    );
    count(
        t.links_removed,
        format!("-{}", plural(t.links_removed, "link", "links")),
    );
    count(
        t.blocks_written,
        format!("{} written", plural(t.blocks_written, "block", "blocks")),
    );
    count(
        t.blocks_removed,
        format!("{} removed", plural(t.blocks_removed, "block", "blocks")),
    );
    count(
        t.grants_added,
        format!("+{}", plural(t.grants_added, "grant", "grants")),
    );
    count(
        t.grants_removed,
        format!("-{}", plural(t.grants_removed, "grant", "grants")),
    );
    if t.exclude_changed {
        parts.push("exclude updated".to_owned());
    }
    (!parts.is_empty()).then(|| parts.join(", "))
}

/// The changes, warnings and foreign entries of `report`, indented under its heading.
fn details(report: &Report, dry_run: bool, out: &Output) -> String {
    let p = &out.paint;
    let mut text = String::new();
    if dry_run {
        for c in &report.changes {
            text.push_str(&p.dim(format!(
                "    would {} {} {}\n",
                c.op.as_str(),
                c.what.as_str(),
                out.path(&c.path)
            )));
        }
        for g in &report.grants {
            text.push_str(&diff::unified(&out.path(&g.path), &g.before, &g.after, p));
        }
    }
    for s in &report.skipped {
        text.push_str(&format!(
            "  {} {}: {}\n    {} {}\n",
            p.warn("warning:"),
            out.path(&s.path),
            s.message,
            p.hint("hint:"),
            s.hint
        ));
    }
    for f in &report.foreign {
        text.push_str(&p.dim(format!("  not kipi's, left alone: {}\n", out.path(f))));
    }
    text
}

fn repo(r: &RepoOutcome, dry_run: bool, out: &Output) -> String {
    let p = &out.paint;
    let name = if r.worktree {
        format!("{} {}", p.heading(&r.name), p.dim("(worktree)"))
    } else {
        p.heading(&r.name)
    };
    let path = r.path.as_deref().map(|d| out.path(d));
    let mut text = match path {
        Some(path) => format!("{name}  {path}\n"),
        None => format!("{name}\n"),
    };
    match &r.state {
        RepoState::NotCloned => {
            text.push_str(&p.dim("  skipped: not cloned on this machine\n"));
            return text;
        }
        RepoState::CheckoutMissing => {
            text.push_str(&format!(
                "  {} the checkout is gone; skipped\n    {} re-run `kipi scan` to record where it lives now\n",
                p.warn("warning:"),
                p.hint("hint:")
            ));
            return text;
        }
        RepoState::Orphan(report) => text.push_str(&format!(
            "  {}; its entry is in no mounted store\n",
            describe(&Tally::of(report)).unwrap_or_else(|| "nothing left to remove".into())
        )),
        RepoState::Applied(report) => {
            let status = describe(&Tally::of(report)).unwrap_or_else(|| "up to date".into());
            let size = r
                .always_on
                .map(|s| p.dim(format!(" · always-on ~{}/{} tok", s.tokens, s.budget)))
                .unwrap_or_default();
            text.push_str(&format!("  {status}{size}\n"));
        }
    }
    if let Some(size) = r.always_on.filter(|s| s.over()) {
        text.push_str(&format!(
            "  {} always-on text is ~{} tokens, over the budget of {}\n    {} trim the scope's conventions or the registry, or raise `budget_tokens`\n",
            p.warn("warning:"),
            size.tokens,
            size.budget,
            p.hint("hint:")
        ));
    }
    for n in &r.notices {
        text.push_str(&format!(
            "  note: {}\n    {} {}\n",
            n.message,
            p.hint("hint:"),
            n.hint
        ));
    }
    if let Some(report) = r.state.report() {
        text.push_str(&details(report, dry_run, out));
    }
    text.push_str(&commands(r, out));
    text
}

/// One line per sync command: what ran, failed, would run or was skipped; up-to-date
/// commands and `when` skips only with `-v`.
fn commands(r: &RepoOutcome, out: &Output) -> String {
    let p = &out.paint;
    let mut text = String::new();
    let mut warn = |message: String, hint: String| {
        text.push_str(&format!(
            "  {} {message}\n    {} {hint}\n",
            p.warn("warning:"),
            p.hint("hint:")
        ));
    };
    let rerun = || {
        format!(
            "fix it and run `kipi sync {}` again (failed commands always rerun)",
            r.name
        )
    };
    let mut lines = Vec::new();
    for c in &r.commands {
        let name = &c.name;
        match &c.state {
            CommandState::Ran { ok: true, .. } => lines.push(format!("  ran {name}\n")),
            CommandState::Ran { exit, ok: false } => warn(
                match exit {
                    Some(code) => format!("{name} failed (exit {code})"),
                    None => format!("{name} failed (killed by a signal)"),
                },
                rerun(),
            ),
            CommandState::SpawnFailed(why) => warn(format!("{name} failed: {why}"), rerun()),
            CommandState::WouldRun => lines.push(p.dim(format!("  would run {name}\n"))),
            CommandState::Unapproved => warn(
                format!("skipped {name}: not approved on this machine"),
                format!(
                    "review it with `kipi resolve {}`, then run `kipi sync --allow-commands`",
                    r.name
                ),
            ),
            CommandState::Skipped {
                reason: SkipReason::MissingAttribute,
                key,
            } => warn(
                format!("skipped {name}: attribute {key} is not set"),
                format!(
                    "set it with `kipi bind {} --attr {key}=…`, or add `when = \"{key}\"` to skip silently",
                    r.name
                ),
            ),
            CommandState::Skipped {
                reason: SkipReason::When,
                key,
            } if out.verbosity > 0 => {
                lines.push(p.dim(format!("  skipped {name}: when {key} (not set)\n")));
            }
            CommandState::UpToDate if out.verbosity > 0 => {
                lines.push(p.dim(format!("  {name} up to date\n")));
            }
            CommandState::Skipped { .. } | CommandState::UpToDate => {}
        }
    }
    let mut all = lines.concat();
    all.push_str(&text);
    all
}

/// The sync report as text.
pub fn human(outcome: &Outcome, dry_run: bool, out: &Output) -> String {
    let p = &out.paint;
    let mut text = String::new();
    let (mut changed, mut current, mut skipped, mut worktrees) = (0, 0, 0, 0);
    let (mut ran, mut failed, mut due) = (0, 0, 0);
    for r in &outcome.repos {
        worktrees += usize::from(r.worktree);
        for c in &r.commands {
            match &c.state {
                CommandState::Ran { .. } | CommandState::SpawnFailed(_) => ran += 1,
                CommandState::WouldRun => due += 1,
                _ => {}
            }
            failed += usize::from(c.state.failed());
        }
        match r.state.report() {
            Some(report) if report.is_unchanged() => current += 1,
            Some(_) => changed += 1,
            None => skipped += 1,
        }
        text.push_str(&repo(r, dry_run, out));
    }
    for s in &outcome.registries {
        let t = Tally::of(&s.report);
        if t.blocks_written + t.blocks_removed == 0 && s.report.skipped.is_empty() {
            continue;
        }
        let (written, removed) = if dry_run {
            ("to write", "to remove")
        } else {
            ("written", "removed")
        };
        let what = match (t.blocks_written, t.blocks_removed) {
            (w, 0) => format!("{} {written}", plural(w, "registry", "registries")),
            (0, r) => format!("{} {removed}", plural(r, "registry", "registries")),
            (w, r) => format!("{w} {written}, {r} {removed}"),
        };
        text.push_str(&format!(
            "{}  {}  {what}\n",
            p.heading(format!("store {}", s.store)),
            out.path(&s.report.root)
        ));
        text.push_str(&details(&s.report, dry_run, out));
    }
    let total = changed + current + skipped;
    let (lead, verb) = if dry_run {
        ("Dry run over", "would change")
    } else {
        ("Synced", "changed")
    };
    let covered = match worktrees {
        0 => plural(total, "repo", "repos"),
        n if n == total => plural(n, "worktree", "worktrees"),
        n => format!(
            "{} and {}",
            plural(total - n, "repo", "repos"),
            plural(n, "worktree", "worktrees")
        ),
    };
    let mut commands = String::new();
    if ran > 0 {
        commands.push_str(&format!(", {} ran", plural(ran, "command", "commands")));
    }
    if failed > 0 {
        commands.push_str(&format!(", {failed} failed"));
    }
    if due > 0 {
        commands.push_str(&format!(
            ", {} would run",
            plural(due, "command", "commands")
        ));
    }
    text.push_str(&format!(
        "\n{lead} {covered}: {changed} {verb}, {current} up to date, {skipped} skipped{commands}.\n",
    ));
    text
}
