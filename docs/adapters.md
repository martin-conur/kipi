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
| Untracked side file | `CLAUDE.local.md`, used only when `CLAUDE.md` is tracked. Current, not deprecated: "It loads alongside `CLAUDE.md` and is treated the same way"; "Within each directory, `CLAUDE.local.md` is appended after `CLAUDE.md`" | <https://code.claude.com/docs/en/memory> |
| Imports | `@path` lines in a memory file load that file; kipi uses `@AGENTS.md` (see below) | <https://code.claude.com/docs/en/memory> |
| Skills | `.claude/skills/<name>` → symlink to `<store>/skills/<name>`; Claude Code reads `SKILL.md` from a symlinked skill directory and loads a skill once even if several locations point at it | <https://code.claude.com/docs/en/skills> |
| Read grant | `.claude/settings.local.json` → `permissions.additionalDirectories` (array of paths); project-local, per-user settings, highest precedence below managed settings and the command line | <https://code.claude.com/docs/en/settings>, <https://code.claude.com/docs/en/permissions> |

**Grant semantics.** "Files in additional directories follow the same permission rules as the
original working directory: they become readable without prompts, and file editing permissions
follow the current permission mode." Under `acceptEdits` that means edits in granted directories are
auto-accepted too, so the grant does widen auto-accepted writes in that mode. The grant gives file
access only: directories in `permissions.additionalDirectories` "grant file access only and don't
load any of the configuration" from those directories (source: permissions page, "Working
directories"). Re-checked for #23: the key and semantics are as above, so kipi writes the grant
there unchanged.

**How kipi merges.** kipi only adds and removes its own entries (the manifest's `[[grant]]`
records them); every other key and every user entry is left alone, and a path the user already
listed is never claimed. Malformed JSON, a tracked or symlinked settings file, and a symlinked
`.claude` are reported and left untouched. A settings file kipi created is deleted again once its
entries are all that is left. Details: `docs/decisions.md`, "Stage 2: read grants (#23)".

**Trust.** `settings.local.json` is applied without the workspace-trust step unless it is tracked
in git or `.claude` is a symlink; then Claude Code holds its additional directories until the
folder is trusted. kipi lists the file in `.git/info/exclude` and never symlinks `.claude` itself,
which keeps the grant effective without a trust prompt.

**AGENTS.md.** With the default `Project instructions` setting, `claude-md-or-agents-md`, Claude
Code reads `AGENTS.md` **only when there is no `CLAUDE.md` or `CLAUDE.local.md`** in the working
directory or above it; otherwise it reads the CLAUDE files only (memory page, AGENTS.md section and
troubleshooting). So the block is not loaded twice by default. Users who opt into
`claude-md-and-agents-md` (read both) would see the block twice when both adapters are enabled; set
`agents = ["claude"]` for such repos.

The hazard is the reverse: in a repo with a user-written `AGENTS.md` and no CLAUDE file, Claude
reads that `AGENTS.md`, and a kipi-created `CLAUDE.md` would make it stop. The planning rule that
prevents this is in `docs/decisions.md` ("Stage 2: adapters (#6)"). The adapter exposes the
behaviour through `reads_when_absent` (`AGENTS.md`) and `import_line` (`@AGENTS.md`).

Only the repo directory is considered: a `CLAUDE.md` in a parent directory also switches Claude to
the CLAUDE files, and kipi does not look above the repo.

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
so the generic adapter writes nothing.

**Grant equivalent (investigated in #23; future work, not implemented).** Checked 2026-10-06
against <https://learn.chatgpt.com/docs/sandboxing> (was
`developers.openai.com/codex/concepts/sandboxing`): the only extra-directory setting it names is
`sandbox_workspace_write.writable_roots` in `config.toml` (the `--add-dir` flag adds to it), and it
grants **write** access. That page does not say outright that reads outside the workspace are
allowed (third-party guides say they are; see "Still to re-verify"), but nothing there offers a
read-only root either. A writable root would let the agent change other repos, which is more than
kipi's grant means, so the generic adapter keeps writing no grant; revisit if Codex adds a
read-only root setting.

**Still to re-verify.** In this check the Codex skills page and Codex sandbox page could not be
fetched (the latter has moved under `learn.chatgpt.com`); the `.agents/skills` location and the
sandbox read behaviour above are carried from the spec (§3) and earlier reading of those pages.
Confirm both before 0.1.0 (#18).

## Policy decisions

Recorded in `docs/decisions.md` under "Stage 2: adapters (#6)": AGENTS.md by default (§15
decision 2) and tracked always-on files (§15 decision 3).
