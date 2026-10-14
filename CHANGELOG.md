# Changelog

All notable changes to kipi are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and kipi uses
[Semantic Versioning](https://semver.org/). The JSON output (`--json`, see `docs/json.md`) is
the stable interface; human-readable output may change in any release.

## [Unreleased]

### Fixed

- **The installer check now runs on every release** (#68). It triggered on `release: published`,
  which never fires for a release published with `GITHUB_TOKEN`, so it only ran by hand. It now
  runs when a tag-push Release run succeeds. `gh workflow run installer.yml -f version=vX.Y.Z`
  still runs it on demand.

## [0.2.0]

### Added

- **Installer check on every release.** A new workflow installs each published release with
  `kipi-cli-installer.sh` on a bare Debian container (x86_64), Linux arm64 and macOS arm64, and
  runs the fresh-machine end to end against the installed binary.

- **Describe how related repos connect** (#62). A `related` item can be an inline table
  `{ repo = "app", via = "marts.* tables consumed by app's loaders" }`, mixed freely with plain
  names. The description is shown next to the repo in the block's "This repo" section, in
  `explain`, in `tree` (truncated to fit) and as `via` in `resolve --json` / `tree --json`.
  `kipi bind <repo> --related <target> --via "<text>"` sets or replaces it without touching
  comments, and `--via ""` removes it. Stores without `via` are unchanged and need no migration;
  the JSON gains a `via` field (`null` when unset) on each related repo.
- **Store sync over git** (#16). For a store that is a git repo, each store syncing on its own
  with its own remote:
  - `kipi init --store <path> --clone <git-url>` clones a store into a new or empty directory,
    validates it and mounts it, then names the next steps (`kipi scan`, `kipi sync --all`).
  - `kipi pull [--store <name>]... [--no-sync]` fetches and fast-forwards each store, then
    re-validates and runs `sync --all`. A diverged store stops with a hint
    (`git pull --rebase`) and is left untouched; the others still pull. Stores that are not git
    repos, or have no upstream, are skipped with a note.
  - `kipi push [--store <name>]... [-m <msg>]` commits every change in the store but `generated/`,
    with a message listing the changed repos, skills and scopes, and pushes. Refused, with
    nothing committed, on an error-level doctor finding in the store, on machine-only state
    inside it, or (for a store named with `--store`) when it has no upstream.
  - `--json` for `pull` and `push` (`docs/json.md`, "Pull and push"). When the sync after a pull
    fails, `pull --json` still prints every store's row, with the sync's `error` and `hint`.

### Changed

- `kipi stores` shows each store's branch against its upstream: `clean` / `dirty`, then
  `3 ahead`, `2 behind` or `no upstream`, and `not a git repo` (was `local, no git`). `--json`
  adds `upstream`, `ahead` and `behind` to `git`. Nothing is fetched. No upgrade step.

## [0.1.1]

### Fixed

- `sync` no longer writes its block into a tracked memory file whose name differs only in
  case, such as `CLAUDE.MD`, `Claude.md` or `agents.md` (#60). On a case-insensitive
  filesystem (the macOS default), git reported `CLAUDE.md` as untracked and kipi appended
  its block to the tracked file. Tracked files now match case-insensitively, so the block goes
  to `CLAUDE.local.md` (Claude Code) or is skipped with a notice (`AGENTS.md`). A repo
  synced with 0.1.0 is repaired by the next `kipi sync`: the block moves out of the tracked
  file and `git status` is clean again.

## [0.1.0]

First public release.

### Added

- **Stores and scopes.** Plain-directory stores (one per trust boundary) holding
  `bindings.toml`, skills, notes and conventions; several can be mounted. Scopes with
  `extends`, nested roots and `remote_patterns`. `machine.toml` records where each repo is
  cloned on this machine. With no config and no store, commands say so and name `kipi init`.
- **Onboarding without hand-edited TOML.** `kipi init [--scope <name> --root <path>...]`,
  `kipi add scope <name> --root <path>... [--extends <s>] [--remote-pattern <prefix>...]
  [--skill <s>...] [--no-registry] [--store <n>]`, then `kipi scan` and `kipi sync --all`. A new
  scope gets a registry unless `--no-registry` is passed or it extends a scope that has one.
- **Resolution** (`kipi resolve`, `kipi explain`): scope, skills, always-on text, registry,
  related repos and read grants for every repo, with the `bindings.toml` line behind each
  decision. Enforces the leak rule across scopes and stores; skills are the one thing a store
  can share (`share_skills = true`, `<store>/<skill>`).
- **Scan** (`kipi scan`): finds checkouts under scope roots, matches them by remote, and drafts
  summaries and aliases offline from manifests and READMEs, marked `# kipi:draft` (also for an
  existing entry when its first checkout is recorded, without touching values already there).
  A checkout whose remote matches another mounted store's `remote_patterns` is never written
  into the wrong store: scan reports it as `foreign-remote` with the command that adds it to
  the right one. Linked worktrees are never recorded as entries.
- **Sync** (`kipi sync`, `kipi status`): writes a marked block into `CLAUDE.md` / `AGENTS.md`
  (or `CLAUDE.local.md` when `CLAUDE.md` is tracked), symlinks skills into `.claude/skills/` and
  `.agents/skills/`, merges read grants into `.claude/settings.local.json`, and records
  everything in `.kipi/manifest.toml` and `.git/info/exclude` (including a `CLAUDE.md` /
  `AGENTS.md` kipi created, so `git add .` does not commit it). Sync twice is a no-op;
  unbinding removes what kipi wrote. With no repo named, `kipi sync` syncs the working tree it
  runs in. Adapters: `claude` (Claude Code) and `generic` (Codex CLI, Cursor and other
  `AGENTS.md` agents).
- **Worktrees.** A linked worktree is the same repo entry as its checkout; `kipi sync <repo>`
  and `kipi sync --all` sync each one too (`worktree` in `kipi sync --json`).
  `kipi hook install|uninstall [repo...|--all]` installs a git `post-checkout` hook that syncs
  each new worktree as `git worktree add` creates it; an existing hook is kept and still runs.
- **Authoring** (`kipi stores`, `kipi add skill|repo|scope`, `kipi scope`, `kipi bind`,
  `kipi unbind`, `kipi remove repo`, `kipi edit`): comment-preserving TOML edits that refuse
  anything that would break resolution. `kipi scope <name>` changes an existing scope's
  registry, roots, parent, remote patterns, skills and always-on files; `kipi edit
  scope:<name>` opens its table. `kipi add repo <name> --path <dir>` records a checkout for an
  existing entry. `kipi remove repo <name> [--notes]` deletes an entry and what kipi wrote into
  its checkout. Every writer takes `--dry-run`.
- **Lookup for agents** (`kipi where`, `kipi grep [--repos]`).
- **Visibility** (`kipi tree`): stores, scopes, repos and skills with token budgets and sync
  state, in text and JSON. A scope with several roots shows the first and a count.
- **Doctor** (`kipi doctor [--fix]`): misclassified repos (including a work remote recorded in
  another store, `K013`), alias collisions, budgets, stale links and grants, drafts awaiting
  review (check codes in `docs/doctor.md`).
- **Shell completions** (`kipi completions <shell>`) for bash, zsh, fish, elvish and
  PowerShell, including repo, alias, skill, scope and store names.
- **Releases** for Linux (static musl) and macOS, x86_64 and aarch64, with a shell installer and
  SHA-256 checksums. Homebrew is planned for a later release.

[Unreleased]: https://github.com/martin-conur/kipi/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/martin-conur/kipi/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/martin-conur/kipi/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/martin-conur/kipi/releases/tag/v0.1.0
