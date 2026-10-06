//! Agent adapters (trait [`AgentAdapter`]): the only agent-specific code. Each adapter
//! stays under 100 lines and keeps its vendor paths and keys in one constant block at
//! the top, so a vendor change is a one-line edit. Verified paths: `docs/adapters.md`.

pub mod claude;
pub mod generic;

use std::path::{Path, PathBuf};

use crate::model::AgentId;
use crate::plan::{Action, Grant, PlannedSkill};
use crate::render::block::RenderedBlock;

/// Maps the tool-neutral plan onto one agent's native locations (roadmap §3, §12).
pub trait AgentAdapter {
    /// `"claude"` | `"generic"`.
    fn id(&self) -> AgentId;
    /// The project skills directory of `repo`.
    fn skills_dir(&self, repo: &Path) -> PathBuf;
    /// The always-on instructions file (`CLAUDE.md` / `AGENTS.md`).
    fn always_on_file(&self, repo: &Path) -> PathBuf;
    /// An untracked file the agent also loads at startup, used in place of a tracked
    /// [`AgentAdapter::always_on_file`]; `None` when the agent has none.
    fn local_always_on_file(&self, repo: &Path) -> Option<PathBuf>;
    /// Another agent's always-on file this agent reads instead while none of its own
    /// exist; `None` when it reads only its own.
    fn reads_when_absent(&self) -> Option<&'static str>;
    /// The line that makes this agent load `file` (repo-relative) from its always-on file.
    fn import_line(&self, file: &str) -> Option<String>;
    /// The block text written between (and including) the markers.
    fn render_block(&self, block: &RenderedBlock) -> String;
    /// The link that exposes `skill` in `repo`.
    fn render_skill_entry(&self, skill: &PlannedSkill, repo: &Path) -> Action;
    /// The read grant for `repo`; `None` = the agent has no grant mechanism.
    fn render_grant(&self, repo: &Path, grant: &Grant) -> Option<Action>;
    /// What this adapter writes, as `.git/info/exclude` patterns (repo-relative).
    fn exclude_patterns(&self) -> &'static [&'static str];
}

/// The adapter for `id`.
pub fn adapter(id: AgentId) -> &'static dyn AgentAdapter {
    match id {
        AgentId::Claude => &claude::Claude,
        AgentId::Generic => &generic::Generic,
    }
}

/// A symlink `<skills_dir>/<name>` → the skill's directory in its store.
fn skill_link(skills_dir: PathBuf, skill: &PlannedSkill) -> Action {
    Action::Link {
        link: skills_dir.join(&skill.name),
        target: skill.dir.clone(),
    }
}
