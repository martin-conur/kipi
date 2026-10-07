# Changelog

All notable changes to kipi are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and kipi uses
[Semantic Versioning](https://semver.org/). The JSON output (`--json`, see `docs/json.md`) is
the stable interface; human-readable output may change in any release.

## [Unreleased]

## [0.1.0]

First public release.

### Added

- **Stores and scopes.** Plain-directory stores (one per trust boundary) holding
  `bindings.toml`, skills, notes and conventions; several can be mounted. Scopes with
  `extends`, nested roots and `remote_patterns`. `machine.toml` records where each repo is
  cloned on this machine.
- **Resolution** (`kipi resolve`, `kipi explain`): scope, skills, always-on text, registry,
  related repos and read grants for every repo, with the `bindings.toml` line behind each
  decision. Enforces the leak rule across scopes and stores; skills are the one thing a store
  can share (`share_skills = true`, `<store>/<skill>`).
- **Scan** (`kipi scan`): finds checkouts under scope roots, matches them by remote, and drafts
  summaries and aliases offline from manifests and READMEs, marked `# kipi:draft`.
- **Sync** (`kipi sync`, `kipi status`): writes a marked block into `CLAUDE.md` / `AGENTS.md`
  (or `CLAUDE.local.md` when `CLAUDE.md` is tracked), symlinks skills into `.claude/skills/` and
  `.agents/skills/`, merges read grants into `.claude/settings.local.json`, and records
  everything in `.kipi/manifest.toml` and `.git/info/exclude`. Sync twice is a no-op; unbinding
  removes what kipi wrote. Adapters: `claude` (Claude Code) and `generic` (Codex CLI, Cursor and
  other `AGENTS.md` agents).
- **Authoring** (`kipi init`, `kipi stores`, `kipi add skill|repo`, `kipi bind`,
  `kipi unbind`, `kipi edit`): comment-preserving TOML edits that refuse anything that would
  break resolution. Every writer takes `--dry-run`.
- **Lookup for agents** (`kipi where`, `kipi grep [--repos]`).
- **Visibility** (`kipi tree`): stores, scopes, repos and skills with token budgets and sync
  state, in text and JSON.
- **Doctor** (`kipi doctor [--fix]`): misclassified repos, alias collisions, budgets, stale
  links and grants, drafts awaiting review (check codes in `docs/doctor.md`).
- **Shell completions** (`kipi completions <shell>`) for bash, zsh, fish, elvish and
  PowerShell, including repo, alias, skill, scope and store names.
- **Releases** for Linux (static musl) and macOS, x86_64 and aarch64, with a shell installer and
  SHA-256 checksums. Homebrew is planned for a later release.

[Unreleased]: https://github.com/martin-conur/kipi/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/martin-conur/kipi/releases/tag/v0.1.0
