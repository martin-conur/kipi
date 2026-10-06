//! Claude Code adapter: `CLAUDE.md`, `.claude/skills/`, `.claude/settings.local.json` grant.

use std::path::{Path, PathBuf};

use super::{AgentAdapter, skill_link};
use crate::model::AgentId;
use crate::plan::{Action, Grant, PlannedSkill};
use crate::render::block::RenderedBlock;

// Vendor paths and keys (checked against Claude Code docs, see docs/adapters.md).
/// Always-on project memory.
const ALWAYS_ON: &str = "CLAUDE.md";
/// Personal project memory, loaded alongside `CLAUDE.md` and meant to stay untracked.
const LOCAL_ALWAYS_ON: &str = "CLAUDE.local.md";
/// Read in place of `CLAUDE.md` when neither Claude file exists (`claude-md-or-agents-md`).
const READS_WHEN_ABSENT: &str = "AGENTS.md";
/// Memory import syntax: `@path`.
const IMPORT_PREFIX: &str = "@";
/// Project skills: `.claude/skills/<name>/SKILL.md`.
const SKILLS_DIR: &str = ".claude/skills";
/// Per-user project settings, not committed.
const SETTINGS: &str = ".claude/settings.local.json";
/// Extra directories Claude Code may read without prompting.
const GRANT_KEY: &[&str] = &["permissions", "additionalDirectories"];
/// What this adapter writes, for `.git/info/exclude`.
const EXCLUDE: &[&str] = &[".claude/skills/", ".claude/settings.local.json"];

/// The Claude Code adapter.
#[derive(Debug, Clone, Copy)]
pub struct Claude;

impl AgentAdapter for Claude {
    fn id(&self) -> AgentId {
        AgentId::Claude
    }

    fn skills_dir(&self, repo: &Path) -> PathBuf {
        repo.join(SKILLS_DIR)
    }

    fn always_on_file(&self, repo: &Path) -> PathBuf {
        repo.join(ALWAYS_ON)
    }

    fn local_always_on_file(&self, repo: &Path) -> Option<PathBuf> {
        Some(repo.join(LOCAL_ALWAYS_ON))
    }

    fn reads_when_absent(&self) -> Option<&'static str> {
        Some(READS_WHEN_ABSENT)
    }

    fn import_line(&self, file: &str) -> Option<String> {
        Some(format!("{IMPORT_PREFIX}{file}"))
    }

    fn render_block(&self, block: &RenderedBlock) -> String {
        block.text()
    }

    fn render_skill_entry(&self, skill: &PlannedSkill, repo: &Path) -> Action {
        skill_link(self.skills_dir(repo), skill)
    }

    /// Always `Some`, even with no directories: an empty grant still tells the apply
    /// engine to drop the entries an earlier sync added.
    fn render_grant(&self, repo: &Path, grant: &Grant) -> Option<Action> {
        Some(Action::MergeGrant {
            file: repo.join(SETTINGS),
            key: GRANT_KEY,
            entries: grant.dirs.iter().map(|d| d.path.clone()).collect(),
        })
    }

    fn exclude_patterns(&self) -> &'static [&'static str] {
        EXCLUDE
    }
}
