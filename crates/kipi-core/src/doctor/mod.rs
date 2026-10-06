//! Doctor checks (roadmap §10), one module per check; each yields
//! `Finding { level, code, repo, message, hint, fix }`.
//!
//! [`diagnose`] runs in two steps. [`prepare`] first takes the stores as loaded and
//! reports the references that would make loading or resolution fail outright (a
//! reference into another store, a skill that does not exist, a `related` repo outside
//! the scope chain), removing each one so the rest still resolves; then [`check`] runs
//! every other check over the resolved [`Plan`]. Anything else that stops resolution (a
//! cycle in `extends`, an unknown scope) has no check of its own and is returned as the
//! error it always was. The filesystem checks (K016–K022) look at each synced checkout's
//! manifest and disk. Codes are stable and documented in `docs/doctor.md`.
//!
//! [`fix`] turns the safe fixes of a set of findings into [`fix::Repairs`] for
//! `doctor --fix`.

mod aliases;
mod budget;
mod checkouts;
mod cross_store;
mod exclude;
pub mod fix;
mod gone;
mod grants;
mod home_paths;
mod links;
mod missing_skills;
mod notes_size;
mod prepare;
mod registry_size;
mod remote_patterns;
mod root;
mod scope_crossing;
mod skill_description;
mod summary;
mod tracked_block;
mod unscoped;
mod wrong_store;

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use crate::apply::Target;
use crate::cache::Caches;
use crate::error::{Location, Result};
use crate::git::{self, TrackedCache};
use crate::manifest::Manifest;
use crate::model::{Machine, RemoteUrl, Store, StoreSet};
use crate::plan::{Plan, RepoPlan};
use crate::render::display_path;
use crate::tokens::Counter;

pub use prepare::prepare;

/// How bad a finding is. `doctor` exits 2 when any finding is an [`Level::Error`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// A leak or a broken reference.
    Error,
    /// Probably wrong; worth a look.
    Warn,
    /// Worth knowing; nothing is broken.
    Info,
}

impl Level {
    /// The name used in output.
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Error => "error",
            Level::Warn => "warn",
            Level::Info => "info",
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A check, by its stable code. Codes are never reused or renumbered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Code {
    /// K001: the remote matches a scope's `remote_patterns`, the repo resolved elsewhere.
    RemoteScopeMismatch,
    /// K002: placed in a scope none of whose roots holds the checkout, with no explicit `scope`.
    RootNotAncestor,
    /// K003: a `related` repo or a grant crossing unrelated scopes.
    ScopeCrossing,
    /// K004: two repos of one registry claim the same alias.
    AliasCollision,
    /// K005: a referenced skill does not exist.
    MissingSkill,
    /// K006: an absolute home path in `bindings.toml`, notes or conventions.
    HomePath,
    /// K007: a repo with no scope.
    Unscoped,
    /// K008: a repo with no summary, or a summary still marked draft.
    Summary,
    /// K009: a skill `description` missing or over 1,024 characters.
    SkillDescription,
    /// K010: notes over 1,500 tokens.
    NotesSize,
    /// K011: a registry of over 40 repos.
    RegistrySize,
    /// K012: a reference into another store.
    CrossStore,
    /// K013: the remote matches another store's `remote_patterns`.
    WrongStore,
    /// K014: a checkout whose origin differs from its entry's `remote`, or two
    /// checkouts matching one entry.
    CheckoutMismatch,
    /// K015: an entry without `remote` whose checkout has one.
    MissingRemote,
    /// K016: always-on text over `budget_tokens`.
    Budget,
    /// K017: a `machine.toml [repos]` path that no longer exists.
    CheckoutGone,
    /// K018: a skill link pointing outside every mounted store or at nothing.
    StrayLink,
    /// K019: a read-grant entry kipi wrote that points at a missing path.
    StaleGrant,
    /// K020: a link beside kipi's that the manifest does not list.
    ForeignLink,
    /// K021: kipi's `.git/info/exclude` patterns missing.
    MissingExclude,
    /// K022: a generated block in a file git tracks.
    TrackedBlock,
}

impl Code {
    /// Every check, in code order.
    pub const ALL: [Code; 22] = [
        Code::RemoteScopeMismatch,
        Code::RootNotAncestor,
        Code::ScopeCrossing,
        Code::AliasCollision,
        Code::MissingSkill,
        Code::HomePath,
        Code::Unscoped,
        Code::Summary,
        Code::SkillDescription,
        Code::NotesSize,
        Code::RegistrySize,
        Code::CrossStore,
        Code::WrongStore,
        Code::CheckoutMismatch,
        Code::MissingRemote,
        Code::Budget,
        Code::CheckoutGone,
        Code::StrayLink,
        Code::StaleGrant,
        Code::ForeignLink,
        Code::MissingExclude,
        Code::TrackedBlock,
    ];

    /// The stable code, `K001` to `K022`.
    pub fn as_str(self) -> &'static str {
        self.info().0
    }

    /// A short kebab-case name for the check.
    pub fn name(self) -> &'static str {
        self.info().1
    }

    /// The level every finding of this check has.
    pub fn level(self) -> Level {
        self.info().2
    }

    fn info(self) -> (&'static str, &'static str, Level) {
        match self {
            Code::RemoteScopeMismatch => ("K001", "remote-scope-mismatch", Level::Error),
            Code::RootNotAncestor => ("K002", "root-not-ancestor", Level::Warn),
            Code::ScopeCrossing => ("K003", "scope-crossing", Level::Error),
            Code::AliasCollision => ("K004", "alias-collision", Level::Error),
            Code::MissingSkill => ("K005", "missing-skill", Level::Error),
            Code::HomePath => ("K006", "home-path", Level::Error),
            Code::Unscoped => ("K007", "unscoped", Level::Warn),
            Code::Summary => ("K008", "summary", Level::Info),
            Code::SkillDescription => ("K009", "skill-description", Level::Warn),
            Code::NotesSize => ("K010", "notes-size", Level::Info),
            Code::RegistrySize => ("K011", "registry-size", Level::Info),
            Code::CrossStore => ("K012", "cross-store", Level::Error),
            Code::WrongStore => ("K013", "wrong-store", Level::Error),
            Code::CheckoutMismatch => ("K014", "checkout-mismatch", Level::Warn),
            Code::MissingRemote => ("K015", "missing-remote", Level::Info),
            Code::Budget => ("K016", "budget", Level::Warn),
            Code::CheckoutGone => ("K017", "checkout-gone", Level::Warn),
            Code::StrayLink => ("K018", "stray-link", Level::Warn),
            Code::StaleGrant => ("K019", "stale-grant", Level::Warn),
            Code::ForeignLink => ("K020", "foreign-link", Level::Info),
            Code::MissingExclude => ("K021", "missing-exclude", Level::Warn),
            Code::TrackedBlock => ("K022", "tracked-block", Level::Info),
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One problem doctor found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// How bad it is (always `code.level()`).
    pub level: Level,
    /// Which check found it.
    pub code: Code,
    /// The repo entry it is about; `None` for a store-level finding (a skill, a file).
    pub repo: Option<String>,
    /// The declaration or file it points at.
    pub at: Option<Location>,
    /// What is wrong.
    pub message: String,
    /// What to do about it.
    pub hint: String,
    /// A fix `doctor --fix` may apply.
    pub fix: Option<Fix>,
}

impl Finding {
    /// A finding of `code`, at the level the code has.
    pub fn new(
        code: Code,
        repo: Option<&str>,
        at: Option<Location>,
        message: impl Into<String>,
        hint: impl Into<String>,
    ) -> Self {
        Finding {
            level: code.level(),
            code,
            repo: repo.map(str::to_owned),
            at,
            message: message.into(),
            hint: hint.into(),
            fix: None,
        }
    }

    /// The same finding, with a safe fix.
    pub fn with_fix(mut self, fix: Fix) -> Self {
        self.fix = Some(fix);
        self
    }
}

/// A safe fix, applied by `doctor --fix` ([`fix::Repairs`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fix {
    /// Write `remote = "<remote>"` into `[repos.<repo>]` of store `store`.
    FillRemote {
        /// The store owning the entry.
        store: String,
        /// The entry.
        repo: String,
        /// The checkout's normalized origin.
        remote: RemoteUrl,
    },
    /// Remove `"<entry>" = "<path>"` from `machine.toml [repos]`.
    RemoveCheckout {
        /// The entry name.
        entry: String,
        /// The path that no longer exists.
        path: PathBuf,
    },
    /// Remove the broken symlink `link` (still pointing at `target`); kipi did not create it.
    RemoveLink {
        /// The link (absolute).
        link: PathBuf,
        /// What it points at.
        target: PathBuf,
    },
    /// Re-sync the repo entry through the apply engine (`kipi sync <repo>`).
    Resync {
        /// The entry.
        repo: String,
    },
}

impl Fix {
    /// One line saying what the fix does; `home` is shown as `~`.
    pub fn describe(&self, home: Option<&Path>) -> String {
        match self {
            Fix::FillRemote {
                store,
                repo,
                remote,
            } => format!("fill `remote = \"{remote}\"` in [repos.{repo}] of store `{store}`"),
            Fix::RemoveCheckout { entry, path } => format!(
                "remove `{entry}` ({}) from machine.toml [repos]",
                display_path(path, home)
            ),
            Fix::RemoveLink { link, .. } => {
                format!("remove the broken link {}", display_path(link, home))
            }
            Fix::Resync { repo } => format!("re-sync `{repo}`"),
        }
    }
}

/// What doctor knows about one `machine.toml [repos]` checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckoutInfo {
    /// The checkout directory.
    pub path: PathBuf,
    /// Its normalized `remote.origin.url`; `None` without a network origin.
    pub origin: Option<RemoteUrl>,
}

/// The `machine.toml [repos]` checkouts on this machine, by entry name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Checkouts {
    /// Entry name → checkout, for the checkouts that are directories.
    pub by_entry: BTreeMap<String, CheckoutInfo>,
    /// Entry name → recorded path, for the ones that are not (K017).
    pub missing: BTreeMap<String, PathBuf>,
}

impl Checkouts {
    /// Read the origin of every `machine.toml [repos]` checkout that is a directory, and
    /// note the ones that are not. A directory git cannot read, or an origin that is a
    /// local path, counts as having no origin.
    pub fn read(machine: &Machine) -> Self {
        let missing = machine
            .repos
            .iter()
            .filter(|r| !r.path.is_dir())
            .map(|r| (r.entry.clone(), r.path.clone()))
            .collect();
        let by_entry = machine
            .repos
            .iter()
            .filter(|r| r.path.is_dir())
            .map(|r| {
                let origin = git::origin_url(&r.path)
                    .ok()
                    .flatten()
                    .and_then(|raw| RemoteUrl::parse(&raw).ok());
                let info = CheckoutInfo {
                    path: r.path.clone(),
                    origin,
                };
                (r.entry.clone(), info)
            })
            .collect();
        Checkouts { by_entry, missing }
    }

    /// The checkout of entry `name`.
    pub fn get(&self, name: &str) -> Option<&CheckoutInfo> {
        self.by_entry.get(name)
    }
}

/// Everything the checks look at.
#[derive(Debug, Clone, Copy)]
pub struct Inputs<'a> {
    /// The mounted stores, as resolved (after [`prepare`] removed bad references).
    pub stores: &'a StoreSet,
    /// `machine.toml`.
    pub machine: &'a Machine,
    /// The resolved plan.
    pub plan: &'a Plan,
    /// The checkouts that exist on this machine.
    pub checkouts: &'a Checkouts,
    /// The home directory, for the home-path check.
    pub home: &'a Path,
    /// Only findings about this repo entry.
    pub only: Option<&'a str>,
    /// Counts tokens for the size checks.
    pub tokens: &'a Counter,
    /// Which files git tracks, for K022.
    pub tracked: &'a TrackedCache,
    /// Every mounted store's root, as written and resolved ([`store_roots`]), for K018.
    pub store_roots: &'a [PathBuf],
}

/// Each store's root as written and, when it differs, resolved: computed once per run,
/// since K018 compares every link against all of them.
pub fn store_roots(stores: &StoreSet) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for s in stores.stores() {
        let real = std::fs::canonicalize(&s.root).ok();
        for root in std::iter::once(s.root.clone()).chain(real) {
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
    }
    roots
}

/// A repo cloned here that kipi has synced: its plan, checkout and manifest.
pub(crate) struct Synced<'a> {
    pub repo: &'a RepoPlan,
    pub dir: &'a Path,
    pub manifest: Manifest,
}

impl Inputs<'_> {
    /// The plans the per-repo checks look at: every repo, or just `only`.
    fn repos(&self) -> impl Iterator<Item = &RepoPlan> {
        self.plan
            .repos
            .iter()
            .filter(|r| self.only.is_none_or(|only| r.name == only))
    }

    /// Whether store-level findings (about a skill or a file, not a repo) are wanted.
    fn store_level(&self) -> bool {
        self.only.is_none()
    }

    /// The repos the filesystem checks look at: cloned here, with a manifest. A manifest
    /// that does not load is skipped; `kipi sync` reports it with its own hint.
    fn synced(&self) -> Vec<Synced<'_>> {
        self.repos()
            .filter_map(|repo| {
                let dir = self.checkouts.get(&repo.name)?.path.as_path();
                let path = Target::repo(dir).manifest;
                if !path.is_file() {
                    return None;
                }
                let manifest = Manifest::load(&path).ok()?;
                Some(Synced {
                    repo,
                    dir,
                    manifest,
                })
            })
            .collect()
    }

    /// Whether `path` lies inside a mounted store (compared as written, then resolved).
    fn in_store(&self, path: &Path) -> bool {
        let real = std::fs::canonicalize(path).ok();
        self.stores.stores().iter().any(|s| {
            let root = std::fs::canonicalize(&s.root).ok();
            [Some(&s.root), root.as_ref()]
                .into_iter()
                .flatten()
                .any(|r| path.starts_with(r) || real.as_ref().is_some_and(|p| p.starts_with(r)))
        })
    }

    /// `path` as shown in messages, with the home directory as `~`.
    fn shown(&self, path: &Path) -> String {
        display_path(path, Some(self.home))
    }

    /// The remote that identifies `repo`: its checkout's origin when it has one, else
    /// the entry's `remote`.
    fn remote_of(&self, repo: &RepoPlan) -> Option<&RemoteUrl> {
        let checkout = self
            .checkouts
            .get(&repo.name)
            .and_then(|c| c.origin.as_ref());
        checkout.or_else(|| {
            let (_, entry) = self.stores.repo(&repo.name)?;
            entry.remote.as_ref()
        })
    }
}

/// What [`diagnose`] produced: the stores and plan the checks ran on, and the findings.
#[derive(Debug)]
pub struct Diagnosis {
    /// The stores, minus the references reported as errors.
    pub stores: StoreSet,
    /// The plan resolved from them.
    pub plan: Plan,
    /// Every finding, sorted by level, then code, then repo.
    pub findings: Vec<Finding>,
}

impl Diagnosis {
    /// Whether any finding is an error (`doctor` exits 2).
    pub fn has_errors(&self) -> bool {
        self.findings.iter().any(|f| f.level == Level::Error)
    }
}

/// Run every check over `stores` (each loaded on its own, not yet checked as a set).
/// `only` restricts the findings to one repo entry; store-level findings are then left
/// out. Errors are what stops resolution and has no check of its own.
pub fn diagnose(
    stores: Vec<Store>,
    machine: &Machine,
    checkouts: &Checkouts,
    home: &Path,
    only: Option<&str>,
    caches: &Caches,
) -> Result<Diagnosis> {
    let prepared = prepare(stores, machine)?;
    let roots = store_roots(&prepared.stores);
    let inputs = Inputs {
        stores: &prepared.stores,
        machine,
        plan: &prepared.plan,
        checkouts,
        home,
        only,
        tokens: &caches.tokens,
        tracked: &caches.tracked,
        store_roots: &roots,
    };
    let mut findings: Vec<Finding> = prepared
        .findings
        .into_iter()
        .filter(|f| only.is_none_or(|o| f.repo.as_deref() == Some(o)))
        .collect();
    findings.extend(check(&inputs)?);
    sort(&mut findings);
    Ok(Diagnosis {
        stores: prepared.stores,
        plan: prepared.plan,
        findings,
    })
}

/// Every check that runs on a resolved plan, in code order.
pub fn check(inputs: &Inputs<'_>) -> Result<Vec<Finding>> {
    let mut out = Vec::new();
    remote_patterns::check(inputs, &mut out);
    root::check(inputs, &mut out);
    scope_crossing::check_grants(inputs, &mut out);
    aliases::check(inputs, &mut out);
    home_paths::check(inputs, &mut out)?;
    unscoped::check(inputs, &mut out);
    summary::check(inputs, &mut out);
    skill_description::check(inputs, &mut out);
    notes_size::check(inputs, &mut out)?;
    registry_size::check(inputs, &mut out);
    wrong_store::check(inputs, &mut out);
    checkouts::check(inputs, &mut out);
    budget::check(inputs, &mut out);
    gone::check(inputs, &mut out);
    links::check(inputs, &mut out);
    grants::check(inputs, &mut out);
    exclude::check(inputs, &mut out);
    tracked_block::check(inputs, &mut out);
    Ok(out)
}

/// Errors first, then by code, then by repo (store-level findings first); stable
/// otherwise, so a check's own order is kept.
pub fn sort(findings: &mut [Finding]) {
    findings.sort_by(|a, b| (a.level, a.code, &a.repo).cmp(&(b.level, b.code, &b.repo)));
}
