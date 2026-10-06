# Adapters

kipi's core produces a tool-neutral plan; an adapter maps it onto one agent's native files
(roadmap §3, §12). Adapters are the only agent-specific code. Each lives in
`crates/kipi-core/src/adapters/<id>.rs`, stays under 100 lines (enforced by
`tests/adapters.rs`), and keeps every path and settings key in one constant block at the top, so a
vendor change is a one-line edit.

Vendor locations shift. This page records what each path and key is, where it was checked and
when. Re-check before each release and update the constant block and this page together.

## Claude Code (`claude`)

| What | kipi writes | Source (checked 2026-10-06) |
| --- | --- | --- |
| Always-on file | `CLAUDE.md` in the repo root (Claude Code also walks parent dirs and loads `~/.claude/CLAUDE.md`) | <https://code.claude.com/docs/en/memory> |
| Untracked side file | `CLAUDE.local.md`, used only when `CLAUDE.md` is tracked (see below) | <https://code.claude.com/docs/en/memory> |
| Skills | `.claude/skills/<name>` → symlink to `<store>/skills/<name>`; Claude Code reads `SKILL.md` from a symlinked skill directory and loads a skill once even if several locations point at it | <https://code.claude.com/docs/en/skills> |
| Read grant | `.claude/settings.local.json` → `permissions.additionalDirectories` (array of paths); project-local, per-user settings, highest precedence below managed settings and the command line | <https://code.claude.com/docs/en/settings> |

**Grant semantics.** `additionalDirectories` adds working directories: Claude Code reads files in
them without a prompt. Edits there follow the session's permission mode exactly as edits in the
repo itself do, so the grant does not widen what may be *written* without asking beyond what the
user already allows in the repo. kipi only adds and removes its own entries (the manifest records
them, #23); every other key and every user entry is left alone.

**Claude Code also reads `AGENTS.md`.** The memory docs describe using a repository's
`AGENTS.md` on its own or alongside `CLAUDE.md`. When both files carry the generated block and
Claude Code loads both, the block costs its tokens twice; `doctor`'s budget check (#13) is where
that shows up. Disable one adapter per repo (`agents = ["claude"]`) if it matters.

## Generic: Codex CLI, Cursor and other AGENTS.md agents (`generic`)

| What | kipi writes | Source (checked 2026-10-06) |
| --- | --- | --- |
| Always-on file | `AGENTS.md` in the repo root. Codex reads `~/.codex/AGENTS.md`, then each directory from the git root down to the cwd; the combined text is capped at `project_doc_max_bytes` (32 KiB default) | <https://learn.chatgpt.com/docs/agent-configuration/agents-md> (was `developers.openai.com/codex/guides/agents-md`) |
| Untracked side file | none: `AGENTS.override.md` *replaces* `AGENTS.md` at its level instead of adding to it, so writing the block there would hide the user's instructions | same page |
| Skills | `.agents/skills/<name>` → symlink to `<store>/skills/<name>`, the repo skills location of Codex CLI and other Agent-Skills-compatible tools. Skill format: `SKILL.md` with `name` (≤ 64 chars, lowercase letters, digits, single hyphens, equal to the directory name) and `description` (≤ 1,024 chars) | <https://developers.openai.com/codex/skills> (not re-fetched in this check, see below); format: <https://agentskills.io/specification> |
| Read grant | none written in v1 (`render_grant` returns `None`) | — |

**Sandbox reads.** Codex CLI's default sandboxes (`read-only`, `workspace-write`) restrict
*writes* to the workspace (plus configured writable roots) but let the agent read outside it, so
another repo's path from the registry is readable without a grant. Extra writable roots exist
(`sandbox_workspace_write.writable_roots`, `--add-dir`), but kipi never needs to write elsewhere,
so the generic adapter writes nothing; see #23 for the follow-up investigation.

**Still to re-verify.** In this check the Codex skills page and Codex sandbox page could not be
fetched (the latter has moved under `learn.chatgpt.com`); the `.agents/skills` location and the
sandbox read behaviour above are carried from the spec (§3) and earlier reading of those pages.
Confirm both before 0.1.0 (#18).

The same applies to two Claude Code rows: the memory and settings pages were reachable but only
partly readable in this check. That `CLAUDE.local.md` is still loaded (and not deprecated), and the
exact edit semantics of `additionalDirectories`, are carried from earlier reading; confirm them
before 0.1.0 as well.

## Policy decisions

Recorded in `docs/decisions.md` under "Stage 2: adapters (#6)": AGENTS.md by default (§15
decision 2) and tracked always-on files (§15 decision 3).
