# Changelog

All notable changes to kipi are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and kipi uses
[Semantic Versioning](https://semver.org/). The JSON output (`--json`, see `docs/json.md`) is
the stable interface; human-readable output may change in any release.

## [Unreleased]

### Added

- **`kipi scope <name>`** changes an existing scope in place, keeping comments and checking the
  result before writing: `--registry` / `--no-registry`, `--add-root` / `--remove-root`,
  `--extends <s>` / `--no-extends`, `--skill` / `--remove-skill`, `--always` / `--remove-always`.
- **`kipi remove repo <name> [--notes]`** deletes a repo entry and its `machine.toml` line, and
  removes what kipi wrote into its checkout (and linked worktrees); the next `kipi sync --all`
  drops it from the registries. Refused while another entry lists it in `related`. Its
  `notes.md` is kept unless `--notes`.
- **`kipi edit scope:<name>`** opens the store's `bindings.toml` at the `[scopes.<name>]` line
  and validates on exit, like a repo entry.
- **`kipi add repo <name> --path <dir>`** on an existing entry with no checkout on this machine
  records the checkout (and fills a missing `remote`) instead of refusing.
- **`kipi add scope <name> --root <path>... [--extends <s>] [--no-registry] [--skill <s>...]
  [--store <n>]`** declares a scope in a store's `bindings.toml`, keeping its comments, and
  checks the result before writing (`--dry-run` prints the diff). `kipi init --scope <name>
  --root <path>...` creates a store and its first scope in one step. Onboarding (`init`, `add
  scope`, `scan`, `sync --all`) no longer needs a hand-edited TOML. No migration.

- **Worktrees.** A linked worktree (`git worktree add`) is the same repo entry as its checkout:
  `kipi sync <repo>` and `kipi sync --all` now sync each checkout's worktrees too (each with its
  own `.kipi/manifest.toml`), and `--all` cleans the worktrees of an unmounted store's repos.
  `kipi scan` never records a worktree. `kipi sync --json` repos gain `worktree` (bool).
- **`kipi sync` with no repo named** syncs the working tree it runs in, a checkout or a linked
  worktree of one. Outside every known repo it is still an error, now with a hint naming your
  real entries.
- **`kipi hook install|uninstall [repo...|--all]`** installs a git `post-checkout` hook that runs
  `kipi sync` in each new worktree, so it has the block, skill links and read grant as soon as
  `git worktree add` returns. An existing hook is kept as `post-checkout.pre-kipi` and still
  runs; uninstall puts it back unchanged. No migration; run `kipi hook install --all` to opt in.

### Changed

- **A new scope gets a registry by default.** `kipi add scope` and `kipi init --scope` write
  `registry = true` unless `--no-registry` is passed, or the scope extends one that already has a
  registry (its repos are listed there). Existing scopes are unchanged; turn one on with
  `kipi scope <name> --registry`. No migration.

### Fixed

- **`kipi tree` keeps a scope with many roots on one short line.** A scope with several roots
  shows the first and a count (`(~/code/maps +3)`) instead of every root inline; `--machine`
  still lists them all and `--json` is unchanged. A scope header too wide for its count to line
  up no longer pushes every repo row's token and state columns off-screen.
- **`kipi scan` no longer writes a work repo into the personal store.** A new checkout whose
  remote matches a `remote_patterns` entry of another mounted store than the one its path (or
  `--scope`) places it in is no longer added (with its remote and drafted summary): scan reports
  it as `foreign-remote` with the command that adds it to the right store, and records nothing
  about it. An existing entry in the wrong store no longer gets that remote filled in or its
  drafts refreshed. `--scope` now also places a checkout whose root is in such a store. No
  migration; run `kipi doctor` to find entries an earlier scan already wrote (`K013`).
- **The `CLAUDE.md` / `AGENTS.md` that `kipi sync` creates no longer show as untracked.** A file
  kipi creates (and git does not track) is now listed in kipi's `.git/info/exclude` section as
  `/CLAUDE.md` / `/AGENTS.md`, so `git add .` no longer commits the machine-specific block. A
  file you already had is never excluded; one you start tracking with `git add -f` loses the
  line and is no longer deleted when kipi removes its block. Unbinding or unmounting removes the
  files and their lines. No migration: the next `kipi sync` adds the lines.

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
