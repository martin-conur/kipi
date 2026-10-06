//! `kipi status [repo]`: per repo, its scope, whether it is synced (the desired plan vs
//! the manifest vs the disk, diffed without writing), always-on size vs budget, and how
//! many summaries are still drafts (roadmap §7).

use kipi_core::apply;
use kipi_core::config::Env;
use kipi_core::plan::RepoPlan;
use kipi_core::sync::{self, RepoOutcome, RepoState, Selection};

use super::Loaded;
use crate::error::Failure;
use crate::output::color::styles;
use crate::output::table::Table;
use crate::output::{Output, json};

/// Where a repo stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Disk and manifest match the plan.
    Synced,
    /// Synced before, but a sync would change something.
    Stale,
    /// Cloned here and never synced: nothing kipi writes is there.
    Missing,
    /// Not cloned on this machine.
    NotCloned,
    /// `machine.toml` names a checkout that is gone.
    CheckoutMissing,
}

impl State {
    /// The name used in output and JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            State::Synced => "synced",
            State::Stale => "stale",
            State::Missing => "missing",
            State::NotCloned => "not_cloned",
            State::CheckoutMissing => "checkout_missing",
        }
    }

    /// `state` of a dry-run sync of the repo.
    pub fn of(r: &RepoOutcome) -> State {
        match &r.state {
            RepoState::NotCloned => State::NotCloned,
            RepoState::CheckoutMissing => State::CheckoutMissing,
            RepoState::Applied(_) | RepoState::Orphan(_) if !r.had_manifest => State::Missing,
            RepoState::Applied(report) | RepoState::Orphan(report) if report.is_unchanged() => {
                State::Synced
            }
            RepoState::Applied(_) | RepoState::Orphan(_) => State::Stale,
        }
    }
}

/// One repo's status line.
pub struct Row<'a> {
    /// The plan.
    pub plan: &'a RepoPlan,
    /// The dry-run sync.
    pub outcome: RepoOutcome,
    /// Its state.
    pub state: State,
    /// Its summary is still marked `# kipi:draft`.
    pub draft_summary: bool,
}

/// Diff without writing, then report.
pub fn run(env: &Env, loaded: &Loaded, repo: Option<&str>, out: &Output) -> Result<(), Failure> {
    let plans: Vec<&RepoPlan> = match repo {
        Some(name) => vec![loaded.repo(name)?],
        None => loaded.plan.repos.iter().collect(),
    };
    let opts = sync::Options {
        apply: apply::Options {
            dry_run: true,
            force: false,
        },
        home: Some(env.home.clone()),
        ..sync::Options::default()
    };
    let names = plans.iter().map(|p| p.name.clone()).collect();
    let outcome = sync::run(
        &loaded.stores,
        &loaded.machine,
        &loaded.plan,
        &Selection::Only(names),
        &opts,
    )?;
    let rows: Vec<Row> = plans
        .into_iter()
        .zip(outcome.repos)
        .map(|(plan, outcome)| Row {
            state: State::of(&outcome),
            draft_summary: loaded
                .stores
                .repo(&plan.name)
                .is_some_and(|(_, e)| e.summary_is_draft()),
            plan,
            outcome,
        })
        .collect();
    if out.json {
        let text = json::status(&rows).map_err(|e| Failure::json("the status", e))?;
        return Ok(out.print(&text)?);
    }
    out.print(&human(&rows, out))?;
    Ok(())
}

fn human(rows: &[Row], out: &Output) -> String {
    let p = &out.paint;
    let cell = |s: String| (s, styles::none());
    let mut t = Table::new(0);
    t.row(
        ["REPO", "SCOPE", "STATE", "ALWAYS-ON", "CHECKOUT"].map(|h| (h.to_owned(), styles::name())),
    );
    for r in rows {
        let pending = r
            .outcome
            .state
            .report()
            .map_or(0, |report| report.changes.len());
        let state = match r.state {
            State::Stale => format!("stale ({pending} pending)"),
            State::NotCloned => "not cloned".to_owned(),
            State::CheckoutMissing => "checkout gone".to_owned(),
            s => s.as_str().to_owned(),
        };
        let state_style = match r.state {
            State::Synced => styles::explicit(),
            State::Stale | State::Missing | State::CheckoutMissing => styles::removed(),
            State::NotCloned => styles::source(),
        };
        let size = r.outcome.always_on.map_or_else(String::new, |s| {
            let over = if s.over() { " over" } else { "" };
            format!("~{}/{}{over}", s.tokens, s.budget)
        });
        let checkout = r
            .plan
            .path
            .as_deref()
            .map_or_else(|| "-".to_owned(), |d| out.path(d));
        t.row([
            cell(r.plan.name.clone()),
            cell(
                r.plan
                    .scope
                    .value
                    .clone()
                    .unwrap_or_else(|| "unscoped".into()),
            ),
            (state, state_style),
            cell(size),
            cell(checkout),
        ]);
    }
    let mut text = t.render(p);
    let drafts: Vec<&str> = rows
        .iter()
        .filter(|r| r.draft_summary)
        .map(|r| r.plan.name.as_str())
        .collect();
    if !drafts.is_empty() {
        text.push_str(&format!(
            "\n{} draft {}: {}\n  {} review each and remove its `# kipi:draft` marker\n",
            drafts.len(),
            if drafts.len() == 1 {
                "summary"
            } else {
                "summaries"
            },
            drafts.join(", "),
            p.hint("hint:")
        ));
    }
    let behind = rows
        .iter()
        .filter(|r| matches!(r.state, State::Stale | State::Missing))
        .count();
    if behind > 0 {
        text.push_str(&format!(
            "\n{behind} not in sync\n  {} run `kipi sync --all` (or `kipi sync <repo>`); `--dry-run` shows the changes\n",
            p.hint("hint:")
        ));
    }
    text
}
