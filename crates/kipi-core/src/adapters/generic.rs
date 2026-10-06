//! Generic adapter (Codex CLI, Cursor, others): `AGENTS.md`, `.agents/skills/`, no grant.

use std::path::{Path, PathBuf};

use super::{AgentAdapter, skill_link};
use crate::model::AgentId;
use crate::plan::{Action, Grant, PlannedSkill};
use crate::render::block::RenderedBlock;

// Vendor paths (checked against Codex CLI and Agent Skills docs, see docs/adapters.md).
/// Always-on instructions, read by Codex CLI, Cursor and other AGENTS.md agents.
const ALWAYS_ON: &str = "AGENTS.md";
/// Repo skills for Agent-Skills-compatible tools: `.agents/skills/<name>/SKILL.md`.
const SKILLS_DIR: &str = ".agents/skills";
/// What this adapter writes, for `.git/info/exclude`.
const EXCLUDE: &[&str] = &[".agents/"];

/// The generic AGENTS.md adapter.
#[derive(Debug, Clone, Copy)]
pub struct Generic;

impl AgentAdapter for Generic {
    fn id(&self) -> AgentId {
        AgentId::Generic
    }

    fn skills_dir(&self, repo: &Path) -> PathBuf {
        repo.join(SKILLS_DIR)
    }

    fn always_on_file(&self, repo: &Path) -> PathBuf {
        repo.join(ALWAYS_ON)
    }

    /// None: `AGENTS.override.md` replaces `AGENTS.md` rather than adding to it.
    fn local_always_on_file(&self, _repo: &Path) -> Option<PathBuf> {
        None
    }

    fn reads_when_absent(&self) -> Option<&'static str> {
        None
    }

    /// None: AGENTS.md has no import syntax.
    fn import_line(&self, _file: &str) -> Option<String> {
        None
    }

    fn render_block(&self, block: &RenderedBlock) -> String {
        block.text()
    }

    fn render_skill_entry(&self, skill: &PlannedSkill, repo: &Path) -> Action {
        skill_link(self.skills_dir(repo), skill)
    }

    /// None in v1: no portable read-grant mechanism across AGENTS.md agents.
    fn render_grant(&self, _repo: &Path, _grant: &Grant) -> Option<Action> {
        None
    }

    fn exclude_patterns(&self) -> &'static [&'static str] {
        EXCLUDE
    }
}
