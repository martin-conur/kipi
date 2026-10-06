//! `kipi sync [repo...] [--all]`: plan → diff → apply, then a per-repo summary of links,
//! blocks and grants added or removed, always-on size vs budget, and warnings (roadmap
//! §7, §8). `--dry-run` lists every change, with JSON diffs for the read grant.

use kipi_core::apply::{self, Report};
use kipi_core::config::Env;
use kipi_core::error::closest;
use kipi_core::model::AgentId;
use kipi_core::sync::{self, Outcome, RepoOutcome, RepoState, Selection, Tally};

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
    /// `--dry-run`.
    pub dry_run: bool,
}

/// Sync, then report.
pub fn run(env: &Env, loaded: &Loaded, args: &Args, out: &Output) -> Result<(), Failure> {
    let selection = if args.all {
        Selection::All
    } else if args.repos.is_empty() {
        return Err(Failure::user(
            "no repo to sync",
            "name repo entries (`kipi sync billing-api`) or pass `--all`",
        ));
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
    };
    let _lock = if args.dry_run {
        None
    } else {
        Some(apply::Lock::acquire(&env.config_dir)?)
    };
    let outcome = sync::run(
        &loaded.stores,
        &loaded.machine,
        &loaded.plan,
        &selection,
        &opts,
    )?;
    if out.json {
        let text = json::sync(&outcome, args.dry_run).map_err(|e| Failure::json("the sync", e))?;
        return Ok(out.print(&text)?);
    }
    out.print(&human(&outcome, args.dry_run, out))?;
    Ok(())
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
    let name = p.heading(&r.name);
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
    text
}

fn human(outcome: &Outcome, dry_run: bool, out: &Output) -> String {
    let p = &out.paint;
    let mut text = String::new();
    let (mut changed, mut current, mut skipped) = (0, 0, 0);
    for r in &outcome.repos {
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
    text.push_str(&format!(
        "\n{lead} {}: {changed} {verb}, {current} up to date, {skipped} skipped.\n",
        plural(total, "repo", "repos"),
    ));
    text
}
