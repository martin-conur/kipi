//! `sync` end to end (roadmap §8): plan → diff → apply for every selected repo, plus each
//! mounted store's generated registries. `status` is the same run with
//! [`apply::Options::dry_run`] set: it compares the desired plan with the manifest and the
//! disk and writes nothing.
//!
//! A checkout's linked worktrees (`git worktree add`) are the same repo entry: with
//! [`Options::worktrees`] each gets the repo's plan too, with its own manifest. They share
//! one `info/exclude`, so every working tree of a repo keeps the same section there: the
//! patterns all of them want.
//!
//! With [`Options::run_commands`], each working tree's due sync commands (`on_sync`) run
//! right after its plan is applied; otherwise (and in a dry run) they are only reported.
//!
//! The caller loads the stores and `machine.toml`, resolves the [`Plan`] and, for a run
//! that writes, holds [`apply::Lock`] across the call.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::apply::{self, Op, Report, Target, What};
use crate::cache::Caches;
use crate::error::Result;
use crate::git;
use crate::model::{AgentId, Machine, StoreSet};
use crate::on_sync::{self, Ran, Record, State};
use crate::plan::{self, Action, CommandPlan, Decided, Notice, Plan, RepoPlan, SkipReason, Source};
use crate::render::block;
use crate::tokens::Counter;

/// Which repos a run covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// Every repo entry, plus the checkouts `machine.toml` lists for entries that no
    /// longer exist (an unbound repo, an unmounted store), so their manifests clean up.
    All,
    /// These repo entries, in this order; names not in the plan are ignored (the CLI
    /// rejects them first, with a hint).
    Only(Vec<String>),
    /// One working tree of a repo entry: its checkout or one of its linked worktrees, as
    /// [`locate`] finds it.
    Tree {
        /// The entry name.
        repo: String,
        /// The working tree.
        dir: PathBuf,
    },
}

/// The repo entry whose checkout, or a linked worktree of it, is the working tree
/// holding `dir`, with that working tree; `None` outside every known repo.
pub fn locate<'p>(plan: &'p Plan, dir: &Path) -> Option<(&'p RepoPlan, PathBuf)> {
    let (top, common) = git::working_tree(dir)?;
    plan.repos.iter().find_map(|r| {
        let path = r.path.as_deref()?;
        let real = path.canonicalize().ok()?;
        if real == top {
            Some((r, path.to_path_buf()))
        } else if real.join(".git").canonicalize().ok()? == common {
            Some((r, top.clone()))
        } else {
            None
        }
    })
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
    /// Also apply each selected repo's plan to its linked worktrees (and, with
    /// [`Selection::All`], clean the worktrees of orphaned checkouts).
    pub worktrees: bool,
    /// Run each working tree's due, approved sync commands after applying its plan (never
    /// in a dry run). Only `kipi sync` (and `pull`, which syncs) sets it.
    pub run_commands: bool,
}

/// What [`run`] did to the repos and stores it covered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Outcome {
    /// One per covered working tree: the selection's repos in plan order (or as named),
    /// each checkout followed by its linked worktrees, then any orphaned checkouts sorted
    /// by entry name.
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
    /// The checkout, from `machine.toml`, or the linked worktree.
    pub path: Option<PathBuf>,
    /// `path` is a linked worktree of the entry's checkout.
    pub worktree: bool,
    /// What happened.
    pub state: RepoState,
    /// The checkout had a manifest before the run: kipi had synced it before.
    pub had_manifest: bool,
    /// What adapters declined to do, with hints.
    pub notices: Vec<Notice>,
    /// The always-on size; `None` for an orphan.
    pub always_on: Option<AlwaysOnSize>,
    /// The sync commands planned for this working tree, in plan order.
    pub commands: Vec<CommandOutcome>,
}

/// One sync command in one working tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome {
    /// The command's name.
    pub name: String,
    /// What happened.
    pub state: CommandState,
    /// Its record in `.kipi/on_sync.toml` before this run.
    pub last: Option<Record>,
}

/// What happened to one sync command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandState {
    /// It ran.
    Ran {
        /// The exit code; `None` when killed by a signal.
        exit: Option<i32>,
        /// It exited 0.
        ok: bool,
    },
    /// It could not be started.
    SpawnFailed(String),
    /// It last ran with this argv and succeeded: nothing to do.
    UpToDate,
    /// It is due, but this run does not execute commands (a dry run, `status`).
    WouldRun,
    /// It is not approved on this machine.
    Unapproved,
    /// It does not apply: an attribute is not set.
    Skipped {
        /// `when` (silent) or a placeholder's attribute (a warning).
        reason: SkipReason,
        /// The attribute key.
        key: String,
    },
}

impl CommandState {
    /// It ran and failed, or could not be started.
    pub fn failed(&self) -> bool {
        matches!(
            self,
            CommandState::Ran { ok: false, .. } | CommandState::SpawnFailed(_)
        )
    }
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
    /// Tokens of the generated block plus the effective skills' descriptions
    /// ([`crate::tokens`]: an estimate with the configured encoding).
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
/// step 7).
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

/// Sync `selection` (roadmap §8 steps 2–6). Skipped actions are reported, not errors;
/// errors are I/O failures and unreadable conventions or manifests. `caches` count each
/// repo's always-on size and remember which files git tracks.
///
/// A dry run diffs the repos in parallel (it spends most of its time waiting for `git`);
/// a run that writes goes one repo at a time and stops at the first error.
pub fn run(
    stores: &StoreSet,
    machine: &Machine,
    plan: &Plan,
    selection: &Selection,
    opts: &Options,
    caches: &Caches,
) -> Result<Outcome> {
    let mut outcome = Outcome::default();
    for store in stores.stores() {
        let actions: Vec<_> = plan
            .registries
            .iter()
            .filter(|r| r.store == store.name)
            .map(|r| apply::registry_action(&store.root, r, opts.home.as_deref()))
            .collect();
        let report = apply::run(
            &Target::generated(&store.root),
            &actions,
            opts.apply,
            &caches.tracked,
        )?;
        outcome.registries.push(StoreOutcome {
            store: store.name.clone(),
            report,
        });
    }
    let (repos, only): (Vec<&RepoPlan>, _) = match selection {
        Selection::All => (plan.repos.iter().collect(), None),
        Selection::Only(names) => (names.iter().filter_map(|n| plan.repo(n)).collect(), None),
        Selection::Tree { repo, dir } => (plan.repo(repo).into_iter().collect(), Some(dir)),
    };
    let sync =
        |repo: &&RepoPlan| sync_repo(stores, plan, repo, only.map(PathBuf::as_path), opts, caches);
    if opts.apply.dry_run {
        for trees in parallel_map(&repos, sync) {
            outcome.repos.extend(trees?);
        }
    } else {
        for repo in &repos {
            outcome.repos.extend(sync(repo)?);
        }
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
            if !m.path.is_dir() {
                continue;
            }
            for (dir, worktree) in trees(&m.path, opts.worktrees) {
                let target = Target::repo(&dir);
                if !exists(&target.manifest) {
                    continue;
                }
                if !opts.apply.dry_run {
                    State::default().save(&dir)?;
                }
                outcome.repos.push(RepoOutcome {
                    name: m.entry.clone(),
                    store: None,
                    state: RepoState::Orphan(apply::run(
                        &target,
                        &[],
                        opts.apply,
                        &caches.tracked,
                    )?),
                    path: Some(dir),
                    worktree,
                    had_manifest: true,
                    notices: Vec::new(),
                    always_on: None,
                    commands: Vec::new(),
                });
            }
        }
    }
    Ok(outcome)
}

/// Sync `repo`'s checkout and, with [`Options::worktrees`], its linked worktrees; with
/// `only`, just that working tree. One outcome per working tree covered.
fn sync_repo(
    stores: &StoreSet,
    plan: &Plan,
    repo: &RepoPlan,
    only: Option<&Path>,
    opts: &Options,
    caches: &Caches,
) -> Result<Vec<RepoOutcome>> {
    let conventions = block::load_conventions(repo)?;
    let rendered = block::render(plan, repo, &conventions, opts.home.as_deref());
    let mut out = RepoOutcome {
        name: repo.name.clone(),
        store: Some(repo.store.clone()),
        path: repo.path.clone(),
        worktree: false,
        state: RepoState::NotCloned,
        had_manifest: false,
        notices: Vec::new(),
        commands: Vec::new(),
        always_on: Some(always_on_size(
            stores,
            repo,
            &rendered.text(),
            &caches.tokens,
        )),
    };
    let Some(dir) = &repo.path else {
        return Ok(vec![out]);
    };
    if !dir.is_dir() {
        out.state = RepoState::CheckoutMissing;
        return Ok(vec![out]);
    }
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
    // Every working tree is planned, applied or not, since they share one exclude section.
    let mut planned = Vec::new();
    for (tree, worktree) in trees(dir, true) {
        let checkout = apply::checkout(&tree, opts.allow_tracked, &caches.tracked)?;
        let wanted = plan::tree_actions(repo, &tree, &rendered, &checkout);
        planned.push((tree, worktree, wanted));
    }
    let mut patterns: Vec<String> = Vec::new();
    for (.., wanted) in &planned {
        if let Action::Exclude { patterns: own, .. } = &wanted.exclude {
            for p in own {
                if !patterns.contains(p) {
                    patterns.push(p.clone());
                }
            }
        }
    }
    let real = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let only = only.map(real);
    let mut outcomes = Vec::new();
    for (tree, worktree, mut wanted) in planned {
        let selected = match &only {
            Some(only) => real(&tree) == *only,
            None => !worktree || opts.worktrees,
        };
        if !selected {
            continue;
        }
        if let Action::Exclude { patterns: own, .. } = &mut wanted.exclude {
            own.clone_from(&patterns);
        }
        let target = Target::repo(&tree);
        let mut tree_out = RepoOutcome {
            had_manifest: exists(&target.manifest),
            notices: wanted
                .agents
                .iter()
                .flat_map(|a| a.notices.iter().cloned())
                .collect(),
            path: Some(tree),
            worktree,
            ..out.clone()
        };
        let actions: Vec<_> = wanted.actions().cloned().collect();
        tree_out.state =
            RepoState::Applied(apply::run(&target, &actions, opts.apply, &caches.tracked)?);
        if let Some(tree) = &tree_out.path {
            tree_out.commands = commands(repo, tree, worktree, opts)?;
        }
        outcomes.push(tree_out);
    }
    Ok(outcomes)
}

/// The sync commands of `repo` in its working tree `tree`: every one in the checkout, those
/// with `worktrees = true` in a linked worktree. A `Ready` one runs when it never ran here,
/// its argv changed, its last run failed, or with `--force`; with
/// [`Options::run_commands`] off (or in a dry run) it is only reported. Then the tree's
/// `.kipi/on_sync.toml` keeps a record for each command still planned here.
fn commands(
    repo: &RepoPlan,
    tree: &Path,
    worktree: bool,
    opts: &Options,
) -> Result<Vec<CommandOutcome>> {
    let planned: Vec<_> = repo
        .on_sync
        .iter()
        .filter(|c| !worktree || c.worktrees)
        .collect();
    let execute = opts.run_commands && !opts.apply.dry_run;
    if planned.is_empty() && !(execute && exists(&State::path(tree))) {
        return Ok(Vec::new());
    }
    let mut state = State::load(tree)?;
    let mut out = Vec::with_capacity(planned.len());
    for cmd in &planned {
        let last = state.get(&cmd.name).cloned();
        let s = match &cmd.outcome {
            CommandPlan::Skipped { reason, key } => CommandState::Skipped {
                reason: *reason,
                key: key.clone(),
            },
            CommandPlan::Unapproved { .. } => CommandState::Unapproved,
            CommandPlan::Ready { argv } => {
                let hash = on_sync::argv_hash(argv);
                let due =
                    opts.apply.force || last.as_ref().is_none_or(|r| r.argv_hash != hash || !r.ok);
                if !due {
                    CommandState::UpToDate
                } else if !execute {
                    CommandState::WouldRun
                } else {
                    let env = [
                        ("KIPI_REPO", repo.name.as_str()),
                        ("KIPI_STORE", repo.store.as_str()),
                    ];
                    let ran = on_sync::spawn(argv, tree, &env);
                    let (exit, ok) = match &ran {
                        Ran::Exited { exit, ok } => (*exit, *ok),
                        Ran::SpawnFailed(_) => (None, false),
                    };
                    state.set(Record {
                        name: cmd.name.clone(),
                        argv_hash: hash,
                        ok,
                        exit,
                        at: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map_or(0, |d| d.as_secs()),
                    });
                    match ran {
                        Ran::Exited { exit, ok } => CommandState::Ran { exit, ok },
                        Ran::SpawnFailed(why) => CommandState::SpawnFailed(why),
                    }
                }
            }
        };
        out.push(CommandOutcome {
            name: cmd.name.clone(),
            state: s,
            last,
        });
    }
    if execute {
        state
            .runs
            .retain(|r| planned.iter().any(|c| c.name == r.name));
        state.save(tree)?;
    }
    Ok(out)
}

/// The working trees of the checkout `dir`: itself, then (with `linked`) its linked
/// worktrees, each flagged as one. A worktree git cannot list is left out.
pub(crate) fn trees(dir: &Path, linked: bool) -> Vec<(PathBuf, bool)> {
    let mut trees = vec![(dir.to_path_buf(), false)];
    if linked {
        let found = git::worktrees(dir).unwrap_or_default();
        trees.extend(found.into_iter().map(|w| (w, true)));
    }
    trees
}

/// The rendered block plus the descriptions of the skills the agent indexes at startup:
/// the cost to one agent, so it is computed from the plan, before any `--agents` override
/// (see [`AlwaysOnSize`]). Each description is counted on its own.
pub fn always_on_size(
    stores: &StoreSet,
    repo: &RepoPlan,
    block: &str,
    tokens: &Counter,
) -> AlwaysOnSize {
    let descriptions: u32 = repo
        .skills
        .iter()
        .filter_map(|s| {
            stores
                .store(&s.store)?
                .skill(&s.name)?
                .description
                .as_deref()
        })
        .map(|d| tokens.count(d))
        .fold(0, u32::saturating_add);
    AlwaysOnSize {
        tokens: tokens.count(block).saturating_add(descriptions),
        budget: repo.budget_tokens.value,
    }
}

/// `f` over `items` on up to one thread per core, results in `items`' order.
fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    if threads < 2 || items.len() < 2 {
        return items.iter().map(f).collect();
    }
    let chunk = items.len().div_ceil(threads);
    let f = &f;
    std::thread::scope(|s| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|c| s.spawn(move || c.iter().map(f).collect::<Vec<R>>()))
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
            .collect()
    })
}

pub(crate) fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}
