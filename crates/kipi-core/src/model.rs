//! Domain types: Store, Scope, RepoEntry, Skill, Machine, Config (roadmap §2, §4, §5).
//!
//! These are the validated, typed view of what kipi reads. Each `bindings.toml`,
//! `machine.toml` and `config.toml` also keeps its [`TomlDoc`] so later commands can
//! write back comment-preserving, and every scope, repo and skill records the
//! [`Location`] it was declared at for `explain` / `doctor` provenance.
//!
//! [`StoreSet`] is every mounted store, validated as a set (roadmap §6, Multiple
//! stores); [`RemoteUrl`] and [`match_checkout`] identify a checkout by its remote.

mod remote;
mod store_set;

pub use remote::{Checkout, MatchedBy, RemoteUrl, RepoMatch, match_checkout};
pub use store_set::{RefKind, RefProblem, RefSite, StoreSet};

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::Deserialize;
use toml_edit::DocumentMut;

use crate::error::Location;

/// The only `[store] version` this kipi understands.
pub const STORE_VERSION: i64 = 1;
/// Always-on token budget per repo when no `budget_tokens` is set anywhere.
pub const DEFAULT_BUDGET_TOKENS: u32 = 4000;
/// Adapters rendered when no `agents` is set anywhere.
pub const DEFAULT_AGENTS: [AgentId; 2] = [AgentId::Claude, AgentId::Generic];
/// Read grant when no `grant` is set anywhere.
pub const DEFAULT_GRANT: GrantMode = GrantMode::Related;

/// An agent adapter kipi can render for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentId {
    /// Claude Code: `CLAUDE.md`, `.claude/skills/`, `.claude/settings.local.json`.
    Claude,
    /// Codex CLI, Cursor and others: `AGENTS.md`, `.agents/skills/`.
    Generic,
}

impl AgentId {
    /// Every adapter, in declaration order.
    pub const ALL: [AgentId; 2] = [AgentId::Claude, AgentId::Generic];

    /// The name used in TOML and output.
    pub fn as_str(self) -> &'static str {
        match self {
            AgentId::Claude => "claude",
            AgentId::Generic => "generic",
        }
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Which directories an agent may read without prompting (roadmap §3, §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GrantMode {
    /// Store dirs plus the local paths of `related` repos.
    Related,
    /// Store dirs plus every cloned repo in the registry.
    Registry,
    /// Nothing.
    None,
}

impl GrantMode {
    /// The name used in TOML and output.
    pub fn as_str(self) -> &'static str {
        match self {
            GrantMode::Related => "related",
            GrantMode::Registry => "registry",
            GrantMode::None => "none",
        }
    }
}

impl fmt::Display for GrantMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A skill named in `skills` / `exclude`: bare (`code-review`, resolved in the
/// referencing store) or qualified (`personal/commit-style`, a sharing store's skill).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SkillRef {
    /// The sharing store for a qualified reference.
    pub store: Option<String>,
    /// The skill's directory name.
    pub name: String,
}

impl FromStr for SkillRef {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (store, name) = match s.split_once('/') {
            Some((store, name)) => (Some(store), name),
            None => (None, s),
        };
        if let Some(store) = store {
            check_name(store).map_err(|why| format!("store name in `{s}` {why}"))?;
        }
        check_name(name).map_err(|why| format!("skill name in `{s}` {why}"))?;
        Ok(SkillRef {
            store: store.map(str::to_owned),
            name: name.to_owned(),
        })
    }
}

impl fmt::Display for SkillRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.store {
            Some(store) => write!(f, "{store}/{}", self.name),
            None => f.write_str(&self.name),
        }
    }
}

/// Check a store, scope, repo or skill name: ASCII letters, digits, `-`, `_` and `.`,
/// starting with a letter or digit. `/` and `:` are reserved for qualified references
/// (`store/skill`) and provenance (`store:bindings.toml:12`).
pub fn check_name(name: &str) -> Result<(), &'static str> {
    let mut chars = name.chars();
    match chars.next() {
        None => Err("is empty"),
        Some(c) if !c.is_ascii_alphanumeric() => Err("must start with a letter or digit"),
        _ if chars.all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c)) => Ok(()),
        _ => Err("may only contain letters, digits, `-`, `_` and `.`"),
    }
}

/// The longest `via` a `related` item may carry, in characters.
pub const VIA_MAX_CHARS: usize = 120;

/// Check a `related` item's `via`: non-empty, one line, at most [`VIA_MAX_CHARS`]
/// characters.
pub fn check_via(via: &str) -> Result<(), String> {
    let len = via.chars().count();
    if via.trim().is_empty() {
        Err("is empty".to_owned())
    } else if via.contains(['\n', '\r']) {
        Err("spans several lines".to_owned())
    } else if len > VIA_MAX_CHARS {
        Err(format!(
            "is {len} characters long, over the {VIA_MAX_CHARS} allowed"
        ))
    } else {
        Ok(())
    }
}

/// One item of `repos.<repo>.related`: a repo entry name and, optionally, how the two
/// repos connect (`via`, one direction only).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Related {
    /// The related repo entry name.
    pub name: String,
    /// `via`: a one-line description of the connection.
    pub via: Option<String>,
}

impl From<&str> for Related {
    fn from(name: &str) -> Self {
        Related {
            name: name.to_owned(),
            via: None,
        }
    }
}

/// A value `scan` may have drafted. `is_draft` is true while the value still carries
/// its trailing `# kipi:draft` comment, i.e. no human has reviewed it (roadmap §5.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Drafted<T> {
    /// The value.
    pub value: T,
    /// Still marked `# kipi:draft`.
    pub is_draft: bool,
}

/// A parsed TOML file kept for comment-preserving write-back.
#[derive(Clone)]
pub struct TomlDoc {
    /// Where it was read from.
    pub path: PathBuf,
    /// The document, comments and ordering intact.
    pub doc: DocumentMut,
}

impl fmt::Debug for TomlDoc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TomlDoc")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// One store: a directory with `bindings.toml`, `skills/`, `repos/`, `conventions/`.
#[derive(Debug, Clone)]
pub struct Store {
    /// `[store] name`, unique among mounted stores.
    pub name: String,
    /// The `[store] name` line.
    pub name_at: Location,
    /// `[store] version` (always [`STORE_VERSION`] once loaded).
    pub version: i64,
    /// `[store] share_skills`: other stores may use `<name>/<skill>`.
    pub share_skills: bool,
    /// The store directory.
    pub root: PathBuf,
    /// `[defaults]`.
    pub defaults: Defaults,
    /// `[scopes.*]`, in file order.
    pub scopes: Vec<Scope>,
    /// `[repos.*]`, in file order.
    pub repos: Vec<RepoEntry>,
    /// `skills/*/SKILL.md`, sorted by directory name.
    pub skills: Vec<Skill>,
    /// The parsed `bindings.toml`.
    pub bindings: TomlDoc,
}

impl Store {
    /// The scope declared as `[scopes.<name>]`.
    pub fn scope(&self, name: &str) -> Option<&Scope> {
        self.scopes.iter().find(|s| s.name == name)
    }

    /// The entry declared as `[repos.<name>]`.
    pub fn repo(&self, name: &str) -> Option<&RepoEntry> {
        self.repos.iter().find(|r| r.name == name)
    }

    /// The skill in `skills/<name>/`.
    pub fn skill(&self, name: &str) -> Option<&Skill> {
        self.skills.iter().find(|s| s.name == name)
    }

    /// Tag every location in this store with its name, for `store:file:line` provenance.
    pub(crate) fn tag_locations(&mut self) {
        let name: std::sync::Arc<str> = self.name.as_str().into();
        let tag = |at: &mut Location| at.store = Some(name.clone());
        tag(&mut self.name_at);
        if let Some(at) = &mut self.defaults.at {
            tag(at);
        }
        self.scopes.iter_mut().for_each(|s| tag(&mut s.at));
        self.repos.iter_mut().for_each(|r| tag(&mut r.at));
        self.skills.iter_mut().for_each(|s| tag(&mut s.at));
    }
}

/// `[defaults]`. Unset keys fall back to the built-in defaults via the accessors.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Defaults {
    /// `budget_tokens`.
    pub budget_tokens: Option<u32>,
    /// `agents`.
    pub agents: Option<Vec<AgentId>>,
    /// `grant`.
    pub grant: Option<GrantMode>,
    /// The `[defaults]` header, when present.
    pub at: Option<Location>,
}

impl Defaults {
    /// `budget_tokens`, or [`DEFAULT_BUDGET_TOKENS`].
    pub fn budget_tokens(&self) -> u32 {
        self.budget_tokens.unwrap_or(DEFAULT_BUDGET_TOKENS)
    }

    /// `agents`, or [`DEFAULT_AGENTS`].
    pub fn agents(&self) -> &[AgentId] {
        self.agents.as_deref().unwrap_or(&DEFAULT_AGENTS)
    }

    /// `grant`, or [`DEFAULT_GRANT`].
    pub fn grant(&self) -> GrantMode {
        self.grant.unwrap_or(DEFAULT_GRANT)
    }
}

/// `[scopes.<name>]`: a visibility boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    /// The scope name.
    pub name: String,
    /// `extends`: the parent scope.
    pub extends: Option<String>,
    /// `roots`, with `~` expanded.
    pub roots: Vec<PathBuf>,
    /// `skills`, in order.
    pub skills: Vec<SkillRef>,
    /// `always`: always-on files, relative to the store root.
    pub always: Vec<PathBuf>,
    /// `registry`.
    pub registry: bool,
    /// `remote_patterns`, for doctor's leak check.
    pub remote_patterns: Vec<String>,
    /// `agents` override.
    pub agents: Option<Vec<AgentId>>,
    /// `grant` override.
    pub grant: Option<GrantMode>,
    /// `budget_tokens` override.
    pub budget_tokens: Option<u32>,
    /// The `[scopes.<name>]` header.
    pub at: Location,
}

/// `[repos.<name>]`: one repo's registry record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoEntry {
    /// The entry name, unique across mounted stores.
    pub name: String,
    /// `remote`: normalized origin URL (`host/owner/name`).
    pub remote: Option<RemoteUrl>,
    /// `summary`, with its draft flag.
    pub summary: Option<Drafted<String>>,
    /// `aliases`, with its draft flag (empty and not a draft when unset).
    pub aliases: Drafted<Vec<String>>,
    /// `related` items, in declared order.
    pub related: Vec<Related>,
    /// `skills` added on top of the scope chain's.
    pub skills: Vec<SkillRef>,
    /// `exclude`: skills removed from the effective set.
    pub exclude: Vec<SkillRef>,
    /// `scope`: explicit scope, beats path membership.
    pub scope: Option<String>,
    /// `agents` override.
    pub agents: Option<Vec<AgentId>>,
    /// `grant` override.
    pub grant: Option<GrantMode>,
    /// `budget_tokens` override.
    pub budget_tokens: Option<u32>,
    /// `repos/<name>/notes.md` in the store, when it exists.
    pub notes: Option<PathBuf>,
    /// The `[repos.<name>]` header.
    pub at: Location,
}

impl RepoEntry {
    /// The summary is present and still marked `# kipi:draft`.
    pub fn summary_is_draft(&self) -> bool {
        self.summary.as_ref().is_some_and(|s| s.is_draft)
    }

    /// The alias list is still marked `# kipi:draft`.
    pub fn aliases_is_draft(&self) -> bool {
        self.aliases.is_draft
    }
}

/// `skills/<name>/SKILL.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    /// Frontmatter `name` (equal to the directory name).
    pub name: String,
    /// Frontmatter `description` (doctor warns when missing or too long).
    pub description: Option<String>,
    /// `x-kipi.scope`.
    pub scope: Option<String>,
    /// `x-kipi.tags`.
    pub tags: Vec<String>,
    /// The skill directory.
    pub dir: PathBuf,
    /// The `SKILL.md` file.
    pub at: Location,
}

/// `~/.config/kipi/machine.toml`: this machine's paths. Empty when the file is absent.
#[derive(Debug, Clone, Default)]
pub struct Machine {
    /// `hostname`.
    pub hostname: Option<String>,
    /// `[roots]`: per-scope root overrides.
    pub roots: Vec<MachineRoots>,
    /// `[repos]`: where each entry is cloned here.
    pub repos: Vec<MachineRepo>,
    /// The parsed file, when it exists.
    pub source: Option<TomlDoc>,
}

impl Machine {
    /// The root override for `scope`.
    pub fn roots_for(&self, scope: &str) -> Option<&MachineRoots> {
        self.roots.iter().find(|r| r.scope == scope)
    }

    /// Where `entry` is cloned on this machine.
    pub fn repo_path(&self, entry: &str) -> Option<&Path> {
        self.repos
            .iter()
            .find(|r| r.entry == entry)
            .map(|r| r.path.as_path())
    }
}

/// One `[roots]` line in `machine.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineRoots {
    /// The scope whose roots are overridden.
    pub scope: String,
    /// The roots on this machine.
    pub roots: Vec<PathBuf>,
    /// The line.
    pub at: Location,
}

/// One `[repos]` line in `machine.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineRepo {
    /// The repo entry name.
    pub entry: String,
    /// The checkout on this machine.
    pub path: PathBuf,
    /// The line.
    pub at: Location,
}

/// `~/.config/kipi/config.toml`. Empty when the file is absent.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// `stores`, with `~` expanded, in mount order.
    pub stores: Vec<PathBuf>,
    /// `editor`: fallback when `$EDITOR` is unset.
    pub editor: Option<String>,
    /// `tokenizer`: tiktoken encoding for estimates.
    pub tokenizer: Option<String>,
    /// The parsed file, when it exists.
    pub source: Option<TomlDoc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_refs_parse_bare_and_qualified() {
        let bare: SkillRef = "code-review".parse().unwrap();
        assert_eq!((bare.store, bare.name.as_str()), (None, "code-review"));
        let q: SkillRef = "personal/commit-style".parse().unwrap();
        assert_eq!(q.store.as_deref(), Some("personal"));
        assert_eq!(q.to_string(), "personal/commit-style");
        assert!("a/b/c".parse::<SkillRef>().is_err());
        assert!("/x".parse::<SkillRef>().is_err());
        assert!("".parse::<SkillRef>().is_err());
    }

    #[test]
    fn names() {
        assert!(check_name("acme-ml").is_ok());
        assert!(check_name("v1.2_x").is_ok());
        assert!(check_name("-x").is_err());
        assert!(check_name("a b").is_err());
        assert!(check_name("a:b").is_err());
    }

    #[test]
    fn defaults_fall_back_to_builtins() {
        let d = Defaults::default();
        assert_eq!(d.budget_tokens(), 4000);
        assert_eq!(d.agents(), &[AgentId::Claude, AgentId::Generic]);
        assert_eq!(d.grant(), GrantMode::Related);
    }
}
