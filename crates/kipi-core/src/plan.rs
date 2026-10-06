//! `Plan` and `RepoPlan`: the desired state resolution produces (roadmap §6, §12).
//!
//! A [`Plan`] is tool-neutral. [`repo_actions`] turns one [`RepoPlan`] into the
//! filesystem [`Action`]s each enabled agent adapter wants (roadmap §8 step 3); the apply
//! engine diffs them against the manifest and the disk. Every decision carries a
//! [`Source`] so `explain` can print where it came from.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::adapters::{self, AgentAdapter};
use crate::error::Location;
use crate::model::{AgentId, GrantMode, SkillRef};
use crate::render::block::RenderedBlock;

/// Why a decision was made: the declaration it came from (with its store, file and
/// line) and the rule that applied, e.g. `scopes.acme.skills`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The declaration, or `None` for a built-in default or a fallback that no line states.
    pub at: Option<Location>,
    /// The rule, named after the key that decided it (`repos.events-pipeline.scope`).
    pub rule: String,
}

impl Source {
    /// A decision made by the declaration at `at`.
    pub fn at(at: &Location, rule: impl Into<String>) -> Self {
        Source {
            at: Some(at.clone()),
            rule: rule.into(),
        }
    }

    /// A decision no file states: a built-in default or a fallback.
    pub fn builtin(rule: impl Into<String>) -> Self {
        Source {
            at: None,
            rule: rule.into(),
        }
    }

    /// The store the declaration lives in, if any.
    pub fn store(&self) -> Option<&str> {
        self.at.as_ref().and_then(|at| at.store.as_deref())
    }
}

/// A value and why it is that value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decided<T> {
    /// The value.
    pub value: T,
    /// Where it came from.
    pub source: Source,
}

impl<T> Decided<T> {
    /// `value`, decided by `source`.
    pub fn new(value: T, source: Source) -> Self {
        Decided { value, source }
    }
}

/// The resolved desired state for every repo entry in every mounted store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// One per repo entry, sorted by name.
    pub repos: Vec<RepoPlan>,
    /// One per scope with `registry = true`, sorted by scope name.
    pub registries: Vec<Registry>,
}

impl Plan {
    /// The plan for repo entry `name`.
    pub fn repo(&self, name: &str) -> Option<&RepoPlan> {
        self.repos.iter().find(|r| r.name == name)
    }

    /// The registry built for scope `scope`.
    pub fn registry(&self, scope: &str) -> Option<&Registry> {
        self.registries.iter().find(|r| r.scope == scope)
    }

    /// The registry `repo` renders, if any.
    pub fn registry_of(&self, repo: &RepoPlan) -> Option<&Registry> {
        repo.registry.as_ref().and_then(|r| self.registry(&r.value))
    }
}

/// One repo's resolved state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoPlan {
    /// The entry name.
    pub name: String,
    /// The store owning the entry.
    pub store: String,
    /// The `[repos.<name>]` header.
    pub at: Location,
    /// Where the repo is cloned on this machine (`machine.toml [repos]`); `None` when
    /// it is not cloned here.
    pub path: Option<PathBuf>,
    /// The resolved scope; `None` when unscoped (the source then says why).
    pub scope: Decided<Option<String>>,
    /// The scope's `extends` chain, top-most ancestor first, ending with the scope
    /// itself. Empty when unscoped.
    pub chain: Vec<String>,
    /// Effective skills, in order: scope chain top-down, then the repo's own, minus `exclude`.
    pub skills: Vec<PlannedSkill>,
    /// The repo's `exclude` entries, each naming the skill it removes.
    pub excluded: Vec<PlannedSkill>,
    /// Always-on content, in render order.
    pub always: Vec<AlwaysOn>,
    /// The scope whose registry this repo renders: the nearest in the chain with
    /// `registry = true`.
    pub registry: Option<Decided<String>>,
    /// `related` repos, in declared order.
    pub related: Vec<RelatedRepo>,
    /// The read grant.
    pub grant: Grant,
    /// Adapters to render.
    pub agents: Decided<Vec<AgentId>>,
    /// Always-on token budget.
    pub budget_tokens: Decided<u32>,
}

/// A skill a repo uses (or excludes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedSkill {
    /// The skill's directory name.
    pub name: String,
    /// The store holding the skill.
    pub store: String,
    /// The reference as written (`personal/commit-style`).
    pub reference: SkillRef,
    /// The skill directory.
    pub dir: PathBuf,
    /// Who bound it: a scope of the chain (inherited) or the repo itself (explicit).
    pub origin: SkillOrigin,
    /// The `skills` / `exclude` list that named it.
    pub source: Source,
}

/// Where a [`PlannedSkill`] was bound, for `explain`'s ○ inherited / ● explicit markers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkillOrigin {
    /// Named by `scopes.<scope>.skills`, inherited by every repo in that scope's chain.
    Scope(String),
    /// Named by the repo's own `skills` or `exclude`.
    Repo,
}

impl PlannedSkill {
    /// The skill comes from a store other than the repo's own (a shared skill).
    pub fn is_shared(&self, repo_store: &str) -> bool {
        self.store != repo_store
    }
}

/// One piece of always-on content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlwaysOn {
    /// A scope's `always` file.
    File {
        /// Absolute path inside the owning store.
        path: PathBuf,
        /// The scope listing it.
        scope: String,
        /// The `always` key.
        source: Source,
    },
    /// The registry of `scope`.
    Registry {
        /// The registry's scope.
        scope: String,
        /// The `registry = true` scope.
        source: Source,
    },
    /// The this-repo section (scope, related repos, lookup hints).
    ThisRepo,
}

/// A `related` repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelatedRepo {
    /// The entry name.
    pub name: String,
    /// Its resolved scope.
    pub scope: Option<String>,
    /// Its checkout on this machine, if cloned here.
    pub path: Option<PathBuf>,
    /// Its `notes.md` in the store, if any.
    pub notes: Option<PathBuf>,
    /// The `related` key.
    pub source: Source,
}

/// The directories an agent may read without prompting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    /// `related`, `registry` or `none`, with precedence repo > scope chain > defaults.
    pub mode: Decided<GrantMode>,
    /// Granted directories, without duplicates: store dirs first, then repos by name.
    pub dirs: Vec<GrantDir>,
}

impl Grant {
    /// Whether `path` is granted.
    pub fn contains(&self, path: &Path) -> bool {
        self.dirs.iter().any(|d| d.path == path)
    }
}

/// One granted directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantDir {
    /// Absolute path.
    pub path: PathBuf,
    /// What it is.
    pub kind: GrantKind,
    /// Why it is granted.
    pub source: Source,
}

/// What a granted directory is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantKind {
    /// `repos/` of the owning store (notes).
    StoreRepos,
    /// `skills/` of the owning store, or of a store a shared skill came from.
    StoreSkills {
        /// That store.
        store: String,
    },
    /// The checkout of a repo entry.
    Repo {
        /// The entry name.
        name: String,
    },
}

/// The registry of one scope: every repo in it and its descendants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registry {
    /// The scope with `registry = true`.
    pub scope: String,
    /// The store owning the scope.
    pub store: String,
    /// The scope and its descendants, sorted by name.
    pub scopes: Vec<String>,
    /// Entries sorted by name.
    pub entries: Vec<RegistryEntry>,
    /// Alias keys claimed by more than one entry (doctor reports them as errors).
    pub collisions: Vec<AliasCollision>,
    /// The `registry = true` scope.
    pub source: Source,
}

/// One registry line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryEntry {
    /// The entry name.
    pub name: String,
    /// `aliases`, as written.
    pub aliases: Vec<String>,
    /// The alias list is still a draft.
    pub aliases_draft: bool,
    /// `summary`; `None` lists the repo by name only.
    pub summary: Option<String>,
    /// The summary is still a draft.
    pub summary_draft: bool,
    /// The repo's resolved scope.
    pub scope: String,
    /// The checkout on this machine; `None` renders as `(not cloned here)`.
    pub local: Option<PathBuf>,
    /// `repos/<name>/notes.md`, when it exists.
    pub notes: Option<PathBuf>,
    /// Lookup keys: the name and every alias, case-folded and whitespace-normalized,
    /// without duplicates, in that order.
    pub keys: Vec<String>,
    /// The `[repos.<name>]` header.
    pub source: Source,
}

/// An alias key that several registry entries claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AliasCollision {
    /// The normalized key.
    pub key: String,
    /// The entries claiming it, sorted.
    pub repos: Vec<String>,
}

/// Normalize an alias for matching: case-folded, whitespace runs collapsed to one
/// space, trimmed (roadmap §6, Aliases).
pub fn alias_key(alias: &str) -> String {
    alias
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The `.git/info/exclude` pattern for kipi's own manifest directory (roadmap §4).
pub const MANIFEST_EXCLUDE: &str = ".kipi/";

/// One change the apply engine makes inside a consumer repo. Paths are absolute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// A symlink at `link` pointing at `target` (absolute).
    Link {
        /// Where the symlink goes.
        link: PathBuf,
        /// What it points at.
        target: PathBuf,
    },
    /// Place `text` (begin marker through end marker) into `file` with
    /// [`crate::render::splice::splice_text`].
    WriteBlock {
        /// `CLAUDE.md`, `AGENTS.md`, or the untracked side file.
        file: PathBuf,
        /// The block as the adapter rendered it.
        text: String,
    },
    /// Merge `entries` into the JSON array at `key` in `file`, owning only those entries.
    MergeGrant {
        /// The settings file.
        file: PathBuf,
        /// The path of object keys down to the array.
        key: &'static [&'static str],
        /// The granted directories, in grant order.
        entries: Vec<PathBuf>,
    },
    /// Remove something an earlier sync wrote (produced by the apply diff, never by an adapter).
    Remove {
        /// The path to remove; symlinks are never followed.
        path: PathBuf,
    },
    /// Keep `patterns` in the repo's `.git/info/exclude`.
    Exclude {
        /// The repo checkout.
        repo: PathBuf,
        /// gitignore patterns, relative to the repo root.
        patterns: Vec<String>,
    },
}

/// Something an adapter decided not to do, with what the user can do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// What happened.
    pub message: String,
    /// What to do.
    pub hint: String,
}

/// Facts about the checkout that are not in the plan: which files git tracks.
#[derive(Debug, Clone, Default)]
pub struct Checkout {
    /// Tracked files, as absolute paths.
    pub tracked: BTreeSet<PathBuf>,
    /// `--allow-tracked`: write the block into a tracked file anyway.
    pub allow_tracked: bool,
}

/// What one adapter wants in a repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentActions {
    /// The adapter.
    pub agent: AgentId,
    /// Block, then skill links, then the grant.
    pub actions: Vec<Action>,
    /// What was skipped and why.
    pub notices: Vec<Notice>,
}

/// Everything sync wants in one cloned repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoActions {
    /// The entry name.
    pub repo: String,
    /// The checkout.
    pub dir: PathBuf,
    /// One per enabled adapter, in the repo's `agents` order.
    pub agents: Vec<AgentActions>,
    /// The `.git/info/exclude` patterns of every adapter, then [`MANIFEST_EXCLUDE`].
    pub exclude: Action,
}

impl RepoActions {
    /// Every action, in apply order.
    pub fn actions(&self) -> impl Iterator<Item = &Action> {
        self.agents
            .iter()
            .flat_map(|a| &a.actions)
            .chain(std::iter::once(&self.exclude))
    }
}

/// The actions for `repo`, whose generated block is `block`; `None` when the repo is
/// not cloned on this machine.
pub fn repo_actions(
    repo: &RepoPlan,
    block: &RenderedBlock,
    checkout: &Checkout,
) -> Option<RepoActions> {
    let dir = repo.path.as_ref()?;
    let mut patterns: Vec<String> = Vec::new();
    let agents = repo
        .agents
        .value
        .iter()
        .map(|&id| {
            let adapter = adapters::adapter(id);
            let out = agent_actions(adapter, repo, dir, block, checkout);
            let mut own: Vec<String> = adapter
                .exclude_patterns()
                .iter()
                .map(|p| p.to_string())
                .collect();
            if let Some(Action::WriteBlock { file, .. }) = out.actions.first()
                && *file != adapter.always_on_file(dir)
                && let Ok(rel) = file.strip_prefix(dir)
            {
                own.push(rel.display().to_string());
            }
            for p in own {
                if !patterns.contains(&p) {
                    patterns.push(p);
                }
            }
            out
        })
        .collect();
    patterns.push(MANIFEST_EXCLUDE.to_owned());
    Some(RepoActions {
        repo: repo.name.clone(),
        dir: dir.clone(),
        agents,
        exclude: Action::Exclude {
            repo: dir.clone(),
            patterns,
        },
    })
}

fn agent_actions(
    adapter: &dyn AgentAdapter,
    repo: &RepoPlan,
    dir: &Path,
    block: &RenderedBlock,
    checkout: &Checkout,
) -> AgentActions {
    let mut actions = Vec::new();
    let mut notices = Vec::new();
    match block_file(adapter, dir, checkout) {
        Ok(file) => actions.push(Action::WriteBlock {
            file,
            text: adapter.render_block(block),
        }),
        Err(notice) => notices.push(notice),
    }
    actions.extend(
        repo.skills
            .iter()
            .map(|s| adapter.render_skill_entry(s, dir)),
    );
    actions.extend(adapter.render_grant(dir, &repo.grant));
    AgentActions {
        agent: adapter.id(),
        actions,
        notices,
    }
}

/// Where the block goes (roadmap §15 decision 3): the always-on file unless git tracks
/// it, then the adapter's untracked side file, else nowhere.
fn block_file(
    adapter: &dyn AgentAdapter,
    dir: &Path,
    checkout: &Checkout,
) -> Result<PathBuf, Notice> {
    let main = adapter.always_on_file(dir);
    let tracked = |p: &Path| !checkout.allow_tracked && checkout.tracked.contains(p);
    if !tracked(&main) {
        return Ok(main);
    }
    let name = |p: &Path| {
        p.file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
    };
    match adapter.local_always_on_file(dir) {
        Some(local) if !tracked(&local) => Ok(local),
        Some(local) => Err(Notice {
            message: format!("{} and {} are tracked; generated block not written", name(&main), name(&local)),
            hint: "untrack one of them, or pass `--allow-tracked` to write the block into the tracked file".into(),
        }),
        None => Err(Notice {
            message: format!("{} is tracked; generated block not written", name(&main)),
            hint: "pass `--allow-tracked` to append the block anyway (it will show in `git diff`)".into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alias_keys_fold_case_and_whitespace() {
        assert_eq!(alias_key("  Billing   API\t"), "billing api");
        assert_eq!(alias_key("ÉVÉNEMENTS"), "événements");
        assert_eq!(alias_key("events-pipeline"), "events-pipeline");
    }
}
