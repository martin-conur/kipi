//! `sync` end to end (roadmap §8): plan → diff → apply for every selected repo, plus each
//! mounted store's generated registries. `status` is the same run with
//! [`apply::Options::dry_run`] set: it compares the desired plan with the manifest and the
//! disk and writes nothing.
//!
//! The caller loads the stores and `machine.toml`, resolves the [`Plan`] and, for a run
//! that writes, holds [`apply::Lock`] across the call.

use std::fs;
use std::path::{Path, PathBuf};

use crate::apply::{self, Op, Report, Target, What};
use crate::error::Result;
use crate::model::{AgentId, Machine, StoreSet};
use crate::plan::{self, Decided, Notice, Plan, RepoPlan, Source};
use crate::render::block;
use crate::tokens;

/// Which repos a run covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// Every repo entry, plus the checkouts `machine.toml` lists for entries that no
    /// longer exist (an unbound repo, an unmounted store), so their manifests clean up.
    All,
    /// These repo entries, in this order; names not in the plan are ignored (the CLI
    /// rejects them first, with a hint).
    Only(Vec<String>),
}

/// How [`run`] behaves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    /// `--agents`: render these adapters instead of each repo's `agents`.
    pub agents: Option<Vec<AgentId>>,
    /// `--dry-run` and `--force`.
    pub apply: apply::Options,
    /// `--allow-tracked`: write the block into a tracked always-on file.
    pub allow_tracked: bool,
    /// The home directory, written as `~` in blocks and registries.
    pub home: Option<PathBuf>,
}

/// What [`run`] did to the repos and stores it covered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// One per covered repo: the selection's repos in plan order (or as named), then any
    /// orphaned checkouts sorted by entry name.
    pub repos: Vec<RepoOutcome>,
    /// One per mounted store: its `generated/` directory.
    pub registries: Vec<StoreOutcome>,
}

/// One repo's result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoOutcome {
    /// The entry name.
    pub name: String,
    /// The store owning the entry; `None` for an orphan.
    pub store: Option<String>,
    /// The checkout, from `machine.toml`.
    pub path: Option<PathBuf>,
    /// What happened.
    pub state: RepoState,
    /// The checkout had a manifest before the run: kipi had synced it before.
    pub had_manifest: bool,
    /// What adapters declined to do, with hints.
    pub notices: Vec<Notice>,
    /// The always-on size; `None` for an orphan.
    pub always_on: Option<AlwaysOnSize>,
}

/// What [`run`] did with one repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoState {
    /// The plan was applied (or, in a dry run, diffed).
    Applied(Report),
    /// The entry has no checkout in `machine.toml`.
    NotCloned,
    /// `machine.toml` names a checkout that is not a directory.
    CheckoutMissing,
    /// A checkout `machine.toml` lists for an entry no mounted store has: everything its
    /// manifest records was removed.
    Orphan(Report),
}

impl RepoState {
    /// The apply report, if the repo was applied.
    pub fn report(&self) -> Option<&Report> {
        match self {
            RepoState::Applied(r) | RepoState::Orphan(r) => Some(r),
            RepoState::NotCloned | RepoState::CheckoutMissing => None,
        }
    }
}

/// A repo's always-on text against its budget (roadmap §10): what one agent session
/// loads at startup, per agent. Every adapter writes the same block and links the same
/// skills, so it does not depend on which adapters are enabled (`--agents` included);
/// with no adapter enabled the agent simply finds none of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlwaysOnSize {
    /// Estimated tokens of the generated block plus the effective skills' descriptions.
    pub tokens: u32,
    /// `budget_tokens`.
    pub budget: u32,
}

impl AlwaysOnSize {
    /// Over budget.
    pub fn over(&self) -> bool {
        self.tokens > self.budget
    }
}

/// The changes of one [`Report`] counted the way the sync summary shows them (roadmap §8
/// step 6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    /// Symlinks created or replaced.
    pub links_added: usize,
    /// Symlinks removed.
    pub links_removed: usize,
    /// Generated blocks (or whole generated files) written.
    pub blocks_written: usize,
    /// Generated blocks removed, with the file when kipi had created it.
    pub blocks_removed: usize,
    /// Read-grant entries added.
    pub grants_added: usize,
    /// Read-grant entries removed.
    pub grants_removed: usize,
    /// kipi's `.git/info/exclude` section changed.
    pub exclude_changed: bool,
}

impl Tally {
    /// Count `report`'s changes. In a repo, whole files are only ever block files kipi
    /// created; in a store's `generated/`, they are registries.
    pub fn of(report: &Report) -> Tally {
        let mut t = Tally::default();
        for c in &report.changes {
            match (c.what, c.op) {
                (What::Link, Op::Remove) => t.links_removed += 1,
                (What::Link, _) => t.links_added += 1,
                (What::Block | What::File, Op::Remove) => t.blocks_removed += 1,
                (What::Block | What::File, _) => t.blocks_written += 1,
                (What::Exclude, _) => t.exclude_changed = true,
                (What::Grant | What::Dir, _) => {}
            }
        }
        for g in &report.grants {
            t.grants_added += g.added.len();
            t.grants_removed += g.removed.len();
        }
        t
    }
}

/// One store's generated registries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreOutcome {
    /// The store name.
    pub store: String,
    /// What changed in its `generated/`.
    pub report: Report,
}

/// Sync `selection` (roadmap §8 steps 2–5). Skipped actions are reported, not errors;
/// errors are I/O failures and unreadable conventions or manifests.
pub fn run(
    stores: &StoreSet,
    machine: &Machine,
    plan: &Plan,
    selection: &Selection,
    opts: &Options,
) -> Result<Outcome> {
    let mut outcome = Outcome::default();
    for store in stores.stores() {
        let actions: Vec<_> = plan
            .registries
            .iter()
            .filter(|r| r.store == store.name)
            .map(|r| apply::registry_action(&store.root, r, opts.home.as_deref()))
            .collect();
        let report = apply::run(&Target::generated(&store.root), &actions, opts.apply)?;
        outcome.registries.push(StoreOutcome {
            store: store.name.clone(),
            report,
        });
    }
    let repos: Vec<&RepoPlan> = match selection {
        Selection::All => plan.repos.iter().collect(),
        Selection::Only(names) => names.iter().filter_map(|n| plan.repo(n)).collect(),
    };
    for repo in repos {
        outcome.repos.push(sync_repo(stores, plan, repo, opts)?);
    }
    if *selection == Selection::All {
        let mut orphans: Vec<_> = machine
            .repos
            .iter()
            .filter(|m| plan.repo(&m.entry).is_none())
            // A checkout an entry still owns (say, under a new name) is synced by it.
            .filter(|m| !plan.repos.iter().any(|r| r.path.as_ref() == Some(&m.path)))
            .collect();
        orphans.sort_by(|a, b| a.entry.cmp(&b.entry));
        for m in orphans {
            let target = Target::repo(&m.path);
            if !m.path.is_dir() || !exists(&target.manifest) {
                continue;
            }
            outcome.repos.push(RepoOutcome {
                name: m.entry.clone(),
                store: None,
                path: Some(m.path.clone()),
                state: RepoState::Orphan(apply::run(&target, &[], opts.apply)?),
                had_manifest: true,
                notices: Vec::new(),
                always_on: None,
            });
        }
    }
    Ok(outcome)
}

fn sync_repo(
    stores: &StoreSet,
    plan: &Plan,
    repo: &RepoPlan,
    opts: &Options,
) -> Result<RepoOutcome> {
    let conventions = block::load_conventions(repo)?;
    let rendered = block::render(plan, repo, &conventions, opts.home.as_deref());
    let mut out = RepoOutcome {
        name: repo.name.clone(),
        store: Some(repo.store.clone()),
        path: repo.path.clone(),
        state: RepoState::NotCloned,
        had_manifest: false,
        notices: Vec::new(),
        always_on: Some(always_on_size(stores, repo, &rendered.text())),
    };
    let Some(dir) = &repo.path else {
        return Ok(out);
    };
    if !dir.is_dir() {
        out.state = RepoState::CheckoutMissing;
        return Ok(out);
    }
    let target = Target::repo(dir);
    out.had_manifest = exists(&target.manifest);
    let checkout = apply::checkout(dir, opts.allow_tracked)?;
    let overridden;
    let repo = match &opts.agents {
        Some(agents) => {
            let mut r = repo.clone();
            r.agents = Decided::new(agents.clone(), Source::builtin("--agents"));
            overridden = r;
            &overridden
        }
        None => repo,
    };
    let Some(wanted) = plan::repo_actions(repo, &rendered, &checkout) else {
        return Ok(out);
    };
    out.notices = wanted
        .agents
        .iter()
        .flat_map(|a| a.notices.iter().cloned())
        .collect();
    let actions: Vec<_> = wanted.actions().cloned().collect();
    out.state = RepoState::Applied(apply::run(&target, &actions, opts.apply)?);
    Ok(out)
}

/// The rendered block plus the descriptions of the skills the agent indexes at startup:
/// the cost to one agent, so it is computed from the plan, before any `--agents` override
/// (see [`AlwaysOnSize`]).
fn always_on_size(stores: &StoreSet, repo: &RepoPlan, block: &str) -> AlwaysOnSize {
    let descriptions: String = repo
        .skills
        .iter()
        .filter_map(|s| stores.store(&s.store)?.skill(&s.name)?.description.clone())
        .collect();
    AlwaysOnSize {
        tokens: tokens::estimate(block).saturating_add(tokens::estimate(&descriptions)),
        budget: repo.budget_tokens.value,
    }
}

fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}
