# kipi — Build Spec for Claude Code

Oct 5, 2026 · @Martin Contreras · Rev 2 (registry-first, personal tool, slim v1)

## 1. Purpose and principles

kipi is a single-binary Rust CLI that gives LLM coding agents (Claude Code, Codex CLI, Cursor and any agent that reads SKILL.md / AGENTS.md) a scoped map of all your local git repositories plus your shared skills, without an MCP server or a database. The name is final: binary `kipi`, crate `kipi`, env prefix `KIPI_`.

**Who it is for.** One person working across many repos (some personal, some from an employer) who wants every agent session to know what the other repos are, where they live, and which skills apply, without pasting context or bloating every CLAUDE.md. It is a personal tool anyone can grab; nothing assumes a team, a server or a shared workflow.

**Problem it solves.** Skills get copy-pasted into each repo and drift. Repo A needs to understand repo B, but nothing tells the agent in A what B is or where it lives. Putting everything in every CLAUDE.md clogs the context window. And work knowledge must never leak into personal repos (or the reverse).

**Core bet.** The agent does not need a curated copy of another repo's knowledge; it needs a *map*. A one-line registry entry (name, aliases, summary, local path) is enough for the agent to find the real repo and read its actual code, README and specs, which are always current. kipi maintains the map, links the skills, and grants the agent read access to the repos it needs.

**Design principles**

1. Files are the only interface to agents. Agents get knowledge through the directories and files they already scan; kipi only arranges files. No daemon, no MCP, no DB.
2. The registry is the product. Every repo gets a short, human-reviewed summary and aliases. Deeper knowledge comes from the real repo, read on demand.
3. Progressive disclosure with a budget. Always-on text per repo is capped (default 4,000 tokens, estimated). Everything else is index-then-load (skills) or on-demand (other repos, notes, shell).
4. Scoped, no leakage. Every repo and skill has a scope. Resolution never renders a scope's content, or grants read access, into a repo outside that scope. `doctor` detects misclassified repos.
5. One store per trust boundary. Work knowledge and personal knowledge live in separate stores (plain folders), each synced its own way or not at all, so a personal sync target never carries work content and a personal machine never needs the work store. Nothing crosses stores.
6. The TOML is the source of truth. Any editor works (Helix first). kipi writes back with comment-preserving round-trips.
7. Agent-agnostic. The core produces a tool-neutral plan; thin adapters render it into each agent's native locations.
8. Drafted but supervised. kipi may draft summaries and aliases from READMEs and manifests, but never overwrites text a human has written or edited.

**Non-goals (v1):** MCP server, retrieval server, vector search, TUI, generated repo cards and interface generators, team distribution (committed copies), any built-in sync transport beyond driving `git` for a store that already is a repo (stores are plain folders; git is optional; §11), hosting or registry of skills, editing skill bodies in-tool, Windows support (Linux and macOS only; symlinks required). See §14 for what is deferred and why.

**Hard requirement for the open-source release:** no organization-specific names, paths or examples anywhere in code, docs, fixtures or tests. Examples use `acme`, `~/work/acme`, repos `billing-api`, `events-pipeline`, `ml-platform`, and personal repos under `~/code`.

## 2. Core concepts

Five nouns. Everything in the CLI is an operation on one of them.

| Concept | What it is | Lives in | Reaches the agent via |
| --- | --- | --- | --- |
| Repo entry | One repo's registry record: name, aliases, one-line summary, related repos, scope. Drafted by `scan`, reviewed by a human. | `bindings.toml` `[repos.<name>]` | Always-on (as a registry line) |
| Skill | Procedural "how to do X": a directory with `SKILL.md` (frontmatter + body) and optional `references/`, scripts. Follows the open Agent Skills format so it works unchanged in any compatible agent. | `store/skills/<name>/` | Index-then-load: the agent reads only `description` at startup, the body on demand |
| Notes | Optional hand-written `notes.md` per repo for what the code cannot tell you: invariants, gotchas, "never call v1". Short. | `store/repos/<repo>/notes.md` | On-demand file read; the registry line points at it |
| Scope | A named visibility boundary (`personal`, `acme`, `acme-ml`). Owns skills, always-on conventions and a registry. Scopes can `extends` a parent. | `bindings.toml` | Determines what gets rendered and granted into which repos |
| Registry | Generated one-line-per-repo map for a scope: name, aliases, summary, local path, notes path. | Generated at sync | Always-on, so the agent can resolve "the billing thing" to a repo and a path |

**Three tiers of context cost.** Always-on (conventions, registry, this-repo section) is paid every turn and is budgeted. Indexed (skill descriptions) costs ~50–100 tokens per skill at startup; bodies are free until used. On-demand (other repos' code, notes, `kipi` shell commands) costs zero until the agent reaches for it.

**Related vs ad-hoc.** A declared `related` repo is listed in the repo's own section of the generated block, and its local path is pre-granted for reads, so integration work needs no permission prompts. An ad-hoc mention ("make this match billing") is served by the registry: the agent resolves the alias, gets the path, and reads the real repo (with a permission prompt if it was not pre-granted).

## 3. How agents access knowledge

An agent running inside a repo has exactly four channels. kipi's job is to put the right text behind each channel and to open the right doors.

| Channel | Mechanism | Cost | kipi uses it for |
| --- | --- | --- | --- |
| 1. Startup injection | Files the tool loads unconditionally for a cwd (`CLAUDE.md`, `AGENTS.md`) | Every turn | Scope conventions, registry, this-repo section (≤ budget) |
| 2. Index-then-load | Skill directories; the tool reads frontmatter `description` at startup, body on use | ~50–100 tok per skill | All skills |
| 3. File read by path | The agent's own read tool on any path its permissions allow | 0 until used | Other repos' real code, READMEs and specs; `notes.md` in the store |
| 4. Shell | The agent runs a command | 0 until used | `kipi where`, `kipi grep` for cross-repo lookup without MCP |

**Read permissions (channel 3).** Reading another repo by absolute path must not be blocked or nag on every file. Each adapter may emit a *grant*: the set of directories the agent may read without prompting. The grant for a repo is: its owning store's `repos/` and `skills/` dirs, plus the local paths of its `related` repos (default), or of every repo in its registry (`grant = "registry"`), or nothing (`grant = "none"`). Grants obey the leak rule: a path outside the repo's scope chain, or inside another store (except the `skills/` dir of a store whose shared skill the repo uses), is never granted.

**Per-agent adapters.** The core emits a tool-neutral plan; each adapter maps it to native locations. Adapters are the only agent-specific code and must each be under ~100 lines.

| Agent | Always-on file | Skills dir (project) | Read grant | Notes |
| --- | --- | --- | --- | --- |
| Claude Code | `CLAUDE.md` (also walks parent dirs and `~/.claude/CLAUDE.md`) | `.claude/skills/<name>/SKILL.md` | `.claude/settings.local.json` → `permissions.additionalDirectories` (merged; only kipi's entries are added/removed) | Symlinks fine. `settings.local.json` is per-user and not committed. |
| Generic (Codex CLI, Cursor, others) | `AGENTS.md` | `.agents/skills/<name>/SKILL.md` | none written in v1 | Codex reads outside cwd in its default sandboxes; verify at build time and document. |

Verify each adapter's paths and settings keys against current vendor docs at build time; they shift. Keep paths in one constant block per `adapters/*.rs` file so updates are one-line changes.

**Walkthrough (10 repos: 6 under `~/work/acme`, 4 under `~/code`).** The user opens Claude Code in `events-pipeline`. Startup loads the repo `CLAUDE.md` whose generated block holds the `acme` conventions (~300 tok), the acme registry (6 lines, ~350 tok) and a this-repo section naming `billing-api` as related, plus skill descriptions (~600 tok). Total ≈ 1.3k. The user says "match what billing does for idempotency keys". The registry resolves `billing` → `billing-api` at `~/work/acme/billing-api`, summary mentions `api/openapi.yaml`, notes at `~/work/.kipi-acme/repos/billing-api/notes.md` (the work store). That path is already granted, so the agent reads the notes, then the OpenAPI file and the relevant source directly. Nothing from `~/code` ever appears in this session.

## 4. Store layout

A store is a plain directory. git is optional; kipi never requires a remote and has no sync transport of its own, though for a store that is a git repo `kipi pull` / `kipi push` drive the `git` binary (§11). A machine mounts one or more stores, listed in `~/.config/kipi/config.toml` (default: a single store at `~/.agents/store/`; overridable via `KIPI_STORE` or `--store`, both repeatable).

**One store per trust boundary.** Typical setup: a personal store (`~/.agents/store`, maybe in a personal git remote) mounted on every machine, and a work store (`~/work/.kipi-acme`, kept local or in employer-approved storage) mounted only on the work laptop. Each store owns its scopes, repo entries, skills, notes and conventions; §6 defines what may cross (only explicitly shared skills).

```
~/.agents/store/                    # a store: plain dir; optionally a git repo
├── bindings.toml                   # store name, scopes, repo entries, skills (portable)
├── skills/
│   └── <skill>/
│       ├── SKILL.md                # Agent Skills format; frontmatter + body
│       ├── references/             # optional deep docs
│       └── scripts/                # optional
├── repos/
│   └── <repo>/
│       └── notes.md                # optional, hand-written
├── conventions/
│   └── <scope>.md                  # always-on text per scope (short)
└── generated/                      # gitignored; registry per scope
    └── registry.<scope>.md

~/.config/kipi/
├── config.toml                     # mounted stores, editor, defaults
└── machine.toml                    # THIS machine only: repo paths, root overrides (never in any store)

<repo>/                              # a consumer repo after `sync`
├── CLAUDE.md                        # user content + one generated block (markers)
├── AGENTS.md                        # same block, rendered for generic agents
├── .claude/skills/<skill> -> store  # symlinks
├── .claude/settings.local.json      # kipi merges its read grant into additionalDirectories
├── .agents/skills/<skill> -> store
└── .kipi/                           # manifest of what sync wrote, for clean removal
```

**Gitignore policy.** `sync` adds `.agents/`, `.claude/skills/`, `.claude/settings.local.json`, `.kipi/` to the repo's `.git/info/exclude` (not `.gitignore`), so the consumer repo's tracked files are untouched. If `CLAUDE.md` or `AGENTS.md` is tracked, the generated block shows up in `git diff`; `doctor` warns and the user decides (common choice: keep the block in an untracked `CLAUDE.local.md`-style file — settle in Stage 2, see §15).

## 5. Configuration schema

All files are TOML or Markdown-with-YAML-frontmatter. Parse with `toml_edit` (writes preserve comments and order) and `serde`. Validate on every load; unknown keys are errors (`deny_unknown_fields`) so typos surface.

### 5.1 `bindings.toml` (one per store, portable)

Shown for a work store; a personal store has the same shape with its own scopes (e.g. `personal`).

```toml
[store]
name = "acme"                      # unique among mounted stores; used in qualified refs and output
version = 1                        # schema version; bump on breaking change
share_skills = false               # true lets other mounted stores reference this store's skills as "<store>/<skill>"

[defaults]
budget_tokens = 4000               # always-on cap per repo
agents = ["claude", "generic"]     # adapters to render by default
grant = "related"                  # related | registry | none

[scopes.acme]
roots = ["~/work/acme"]            # path membership; nested roots allowed
skills = ["code-review", "pr-conventions", "personal/commit-style"]   # last one: shared skill from the personal store
always = ["conventions/acme.md"]   # rendered into the generated block
registry = true
remote_patterns = ["github.com/acme-org/"]   # for doctor's leak check

[scopes.acme-ml]
extends = "acme"
roots = ["~/work/acme/ml"]
skills = ["dbt-conventions"]

[repos.billing-api]
remote = "github.com/acme-org/billing-api"   # normalized origin URL; filled by scan; primary match key
summary = "Invoicing and payment orchestration; OpenAPI in api/openapi.yaml"  # kipi:draft
aliases = ["billing", "invoices"]

[repos.acme-api]                   # entry name differs from the dir name `api` to stay unique across stores
remote = "github.com/acme-org/api"
summary = "Public REST gateway"

[repos.events-pipeline]
remote = "github.com/acme-org/events-pipeline"
scope = "acme-ml"                  # optional; explicit beats path
summary = "Kafka consumers that turn raw events into billing and ML features"
aliases = ["events", "pipeline"]
related = ["billing-api", "ml-platform"]
skills = ["stream-recipes"]
exclude = ["pr-conventions"]
agents = ["claude"]                # per-repo override
grant = "registry"                 # per-repo override
```

The personal store's `bindings.toml` (`[store] name = "personal"`, `share_skills = true`) would hold `[scopes.personal] roots = ["~/code"]`, `[repos.side-tool]`, `[repos.api]` (a personal repo also cloned as `api`), and so on.

**Repo identity.** Entry names are unique across all mounted stores. `remote` is the normalized `remote.origin.url` (scheme, user, port and `.git` stripped; `git@host:owner/name` → `host/owner/name`). A repo without a remote is matched by name only. When two repos share a directory name, `scan` names the new entry `<owner>-<name>` and relies on `remote` to match it.

A trailing `# kipi:draft` comment marks a summary or alias list that `scan` drafted and no human has touched. Removing the comment (or editing the value with `kipi edit`) marks it reviewed. `scan --refresh-drafts` may rewrite drafted values; it never touches reviewed ones.

### 5.2 `machine.toml` (per machine, never in any store)

```toml
hostname = "laptop-a"

[roots]                            # override a scope's roots on this machine
"acme" = ["/Volumes/work/acme"]

[repos]                            # where each repo is cloned here; absent = not on this machine
"events-pipeline" = "/Volumes/work/acme/ml/events-pipeline"
"billing-api"     = "/Volumes/work/acme/billing-api"
```

Keys are entry names (unique across stores) for `[repos]` and scope names (unique across stores) for `[roots]`. `kipi scan` populates `[repos]` by walking scope roots (or paths given on the command line) for `.git` directories and matching each to an entry: normalized `remote` first, then directory name (only for entries without a `remote`), then `remote.origin.url` basename. The same repo can live at a different path, or under a different directory name, on every machine. Absolute paths are never written into any store.

### 5.3 `notes.md`

Plain Markdown, no required frontmatter. Recommended sections: `## Invariants`, `## Gotchas`, `## Where to look` (paths relative to the repo root). Target ≤ 500 tokens; `doctor` reports notes over 1,500.

### 5.4 `SKILL.md` frontmatter

Standard Agent Skills fields (`name`, `description`) plus kipi extensions under an `x-kipi` key so the file stays valid for every agent:

```yaml
---
name: dbt-conventions
description: Use when writing or reviewing dbt models, tests or sources in this organization.
x-kipi:
  scope: acme
  tags: [data, sql]
---
```

### 5.5 `config.toml`

```toml
stores = ["~/.agents/store", "~/work/.kipi-acme"]   # mounted stores; a personal machine lists only the first
editor = "hx"                      # fallback $EDITOR
tokenizer = "cl100k_base"          # estimate only; Claude's tokenizer is not public
```

## 6. Resolution semantics

Resolution is a pure function `(stores, machine) -> Plan` with no I/O beyond reading config. It is the only place policy lives; `sync`, `tree`, `explain` and `doctor` all consume the same `Plan`.

**Scope of a repo** (first match wins):

1. `repos.<name>.scope` if set (explicit).
2. The scope whose root (after `machine.toml` overrides) is the deepest ancestor of the repo's path. Nested roots are allowed; deepest wins.
3. For a repo **not cloned on this machine** (no path to compare with roots): the scope whose `remote_patterns` has the longest entry that is a prefix of the repo's normalized `remote`. A tie between scopes decides nothing (fall through). A cloned repo is never placed by pattern, so `doctor`'s "matches a pattern but resolved elsewhere" check keeps its meaning.
4. `personal` if a scope with that name exists, else unscoped (gets no scope content; `doctor` warns).

**Effective skills of a repo:** walk the `extends` chain from the top-most ancestor down, taking the union of each scope's `skills`, then union `repos.<name>.skills`, then remove `repos.<name>.exclude`. Order is preserved for deterministic output. A skill referenced but not present in the repo's store (or, for a qualified `<store>/<skill>`, in that sharing store) is a hard error.

**Effective always-on text:** concatenation, top-most ancestor first, of each scope's `always` files, then the scope's registry (if `registry = true` on the repo's resolved scope or any ancestor; the registry is built for the nearest scope that enables it and includes all repos in that scope and its descendants), then the this-repo section (scope, related repos with paths, retrieval hints).

**Registry entries:** one per repo in the registry's scope set, from `bindings.toml` (`summary`, `aliases`) and `machine.toml` (local path). Repos not cloned on this machine are listed with `local: (not cloned here)`. A repo with no summary is listed with its name only and `doctor` reports it. If `repos/<name>/notes.md` exists, the entry carries its absolute path.

**Related repos:** each `related` entry must resolve to a repo in the same scope chain (same scope, an ancestor, or a descendant). A relation to a repo in an unrelated scope is a hard error; this is the leak rule. Cycles are allowed (A↔B is normal) and are not expanded. An item may be `{ repo = "app", via = "…" }`: a one-line description (≤ 120 characters) of how the repos connect, rendered next to the repo; each direction has its own.

**Read grant:** per `grant` (repo > scope chain > defaults): `related` → the owning store's `repos/` + `skills/` (plus `skills/` of any store a shared skill came from) + local paths of related repos; `registry` → store dirs + local paths of every cloned repo in the registry; `none` → nothing. Every granted path must belong to a repo in the scope chain (leak rule applies). Repos not cloned here are skipped.

**Aliases:** gathered from each entry's `aliases` plus its name. Collisions across repos in the same registry are a `doctor` error (resolution would be ambiguous for the agent). Matching is exact after case-folding and whitespace normalization; no fuzzy matching.

**Precedence:** `repos.<name>.agents` > scope `agents` (walking up) > `defaults.agents`. Same precedence for `budget_tokens` and `grant`.

**Multiple stores.** Each mounted store is loaded independently and then checked as a set:

- Store names, scope names and repo entry names are each unique across all mounted stores; a collision is a hard load error naming both files.
- A scope's `extends`, a repo's `scope`, `related` and `exclude`, and every registry, always-on file, notes file and grant resolve **only within the store that owns the repo**. Any reference into another store is a hard error; this is the cross-store leak rule.
- The single exception is skills: a scope or repo may reference `<store>/<skill>` when that store sets `share_skills = true`. The skill's directory is linked as usual, and the store's `skills/` dir is added to the grant. No other content from the sharing store reaches the repo.
- Bare skill names always resolve within the repo's own store.
- A machine that does not mount a store simply has no entries from it; nothing else changes. Unmounting a store and re-syncing removes everything it contributed, through the manifest.

`kipi explain <repo>` prints every decision above with its source line (`code-review  ← scopes.acme.skills (acme:bindings.toml:12)`).

## 7. CLI command reference

`clap` derive, subcommands below. Global flags: `--store <path>` (repeatable; replaces the configured store list), `--json`, `--no-color`, `-v/-q`. Every command that writes supports `--dry-run`. Exit codes: 0 ok, 1 user error, 2 doctor findings, 3 I/O failure.

| Command | Purpose | Writes |
| --- | --- | --- |
| `init [--store <path>] [--name <n>] [--git] [--scope <s> --root <path>... [--no-registry]]` | Create a store (dirs, `bindings.toml` skeleton with its name, optional `git init`) and add it to `config.toml`; pointed at an existing store, just mounts it; `--scope` also declares a first scope (as `add scope`; `--no-registry` opts out of its registry) | store, config |
| `init --store <path> --clone <git-url> [--name <n>]` | `git clone` a store into a new or empty `<path>`, validate it and mount it; prints the next steps (`scan`, `sync --all`) without running them (§11) | store, config |
| `stores` | List mounted stores: name, path, git state (`clean`/`dirty`, `3 ahead`, `2 behind`, `no upstream`, `not a git repo`; as of the last fetch), scopes, repo count; `--unmount <name>` removes one from `config.toml` | config |
| `pull [--store <name>]... [--no-sync]` | For each mounted store that is a git repo with an upstream: fetch and fast-forward; a diverged store stops with a hint and the others continue; then re-validate and `sync --all` (§11) | stores, repos |
| `push [--store <name>]... [-m <msg>]` | For each mounted store that is a git repo with an upstream: refuse on doctor errors in the store, else commit every change but `generated/` (message lists changed repos, skills, scopes) and `git push` (§11) | stores |
| `scan [path...] [--refresh-drafts]` | Find git repos under scope roots or given paths; write `machine.toml [repos]`; match repos by `remote` then name (§5.2); add missing `[repos.<name>]` entries (with `remote`, drafted summary, aliases) to the store that owns the inferred scope; fill `remote` on existing entries that lack it; report repos whose inferred scope conflicts with `remote_patterns` | machine.toml, bindings.toml of the owning store (new entries / drafts / remote only) |
| `add skill <name> [--scope s] [--store n] [--from <git-url\|path>]` | Create a skill dir from template, or import an existing SKILL.md tree (e.g. from `~/.claude/skills/`) | store |
| `add repo <name> [--scope s] [--path p] [--summary "…"]` | Register a repo by hand in the store owning its scope; reads `remote` from the checkout; on an existing entry with no checkout here, `--path` records one | store, machine |
| `add scope <name> --root <path>... [--extends <s>] [--registry \| --no-registry] [--skill <s>...] [--store <n>]` | Declare `[scopes.<name>]` (toml_edit, comment-preserving) in `--store`, else the store owning `--extends`, else the only mounted store; roots under home written `~/…`; a registry unless `--no-registry` or a scope it extends has one | bindings.toml |
| `scope <name> [--registry \| --no-registry] [--add-root <p>]... [--remove-root <p>]... [--extends <s> \| --no-extends] [--skill <s>]... [--remove-skill <s>]... [--always <f>]... [--remove-always <f>]...` | Change an existing scope in place (comment-preserving, re-validated before writing) | bindings.toml |
| `remove repo <name> [--notes]` | Delete a repo entry and its `machine.toml` path, and remove what kipi wrote into its checkout; refused while another entry's `related` names it; `--notes` also deletes its `notes.md` | store, machine, repos |
| `bind <repo> --skill <s>... / --related <repo>... [--via <text>] / --exclude <s>... / --alias <a>...` | Edit a repo's entry (toml_edit, comment-preserving); `--via` describes the one `--related` relation (`""` removes it) | bindings.toml |
| `unbind <repo> ...` | Inverse of bind | bindings.toml |
| `resolve [repo]` | Print the Plan (JSON with `--json`); no side effects | — |
| `explain <repo>` | Human-readable provenance for every skill, relation, grant and scope decision | — |
| `sync [repo...] [--all] [--agents a,b] [--force]` | Materialize the Plan into repos; remove stale links and grants; rewrite generated blocks; regenerate registries | repos, `generated/` of each store |
| `status [repo]` | Per-repo: scope, synced/stale/missing, always-on tokens vs budget | — |
| `tree [--repo r] [--skill s] [--store n] [--budget] [--machine]` | Tree view of stores → scopes → repos → skills/related with provenance markers (○ inherited, ● explicit, ✗ excluded) | — |
| `where <alias>` | Resolve alias → repo name, summary, local path, notes path (`--notes` prints the notes) | — |
| `grep <pattern> [--scope s] [--repos]` | Regex search over skills, notes and conventions in the repo's own store (plus shared skills it uses); with `--repos`, also over the local checkouts of every repo in the scope chain (gitignore-aware) | — |
| `edit <repo\|skill\|scope:<name>\|notes:<repo>\|conventions:<scope>>` | Open the right file (at the right line for TOML entries and scopes) in `$EDITOR`; validate on exit; editing a drafted entry marks it reviewed | store |
| `doctor [repo] [--fix]` | Checks in §10; `--fix` applies safe fixes | optional |
| `completions <shell>` | Shell completions | — |

**`tree` output sketch**

```
[store acme  ~/work/.kipi-acme  local, no remote]
acme  (~/work/acme)                             always-on: 620 tok
├─ code-review ● pr-conventions ● personal/commit-style ●   [scope]
├─ registry (6 repos, 350 tok)
├─ acme-ml  (~/work/acme/ml)  extends acme     always-on: 700 tok
│  ├─ dbt-conventions ●                                [scope]
│  └─ events-pipeline                           1.3k tok  ✓ synced
│     ├─ skills: code-review ○ pr-conventions ✗ dbt-conventions ○ stream-recipes ●
│     └─ related: billing-api ✓ granted   ml-platform ✓ granted
├─ billing-api                                  1.1k tok  ✓   notes
└─ ml-platform                                  1.0k tok  ✓   draft summary
[store personal  ~/.agents/store  git: clean, shares skills]
personal  (~/code)                              always-on: 280 tok
├─ rust-style ●                                        [scope]
├─ registry (4 repos, 210 tok)
└─ side-tool                                    0.6k tok  ✓
```

`--json` emits the same structure for `jq`/`fzf` pipelines. Colors via `owo-colors`, `NO_COLOR` respected, box-drawing falls back to ASCII when stdout is not a TTY.

## 8. Sync algorithm and generated blocks

`sync` is plan → diff → apply, idempotent, and leaves a manifest so it can undo its own work.

1. Load every mounted store and `machine.toml`; run resolution → `Plan`.
2. For each scope with `registry = true`, render `<owning store>/generated/registry.<scope>.md` (one line per repo: name, aliases, summary, local path or `(not cloned here)`, notes path if any).
3. For each target repo present on this machine, compute the desired state per agent adapter:
   - skill links: `<agent skills dir>/<skill>` → `<store>/skills/<skill>` in the store that owns (or shares) the skill (absolute symlink)
   - generated block content for `CLAUDE.md` / `AGENTS.md`
   - read grant entries (Claude: `additionalDirectories` items in `.claude/settings.local.json`)
4. Read `.kipi/manifest.toml` (what the last sync wrote, including each grant entry it added). Diff desired vs manifest vs filesystem. Anything in the manifest but not desired is removed; anything on disk that kipi did not write is never touched — including user-added `additionalDirectories` entries and every other key in `settings.local.json`.
5. Apply: create dirs, write symlinks (replace only if the existing target differs; refuse to replace a non-symlink file unless `--force`), write generated blocks, merge grant entries into JSON (preserving key order and formatting where possible), update `.git/info/exclude`, write the manifest.
6. Print a summary: per repo, links added/removed, grants added/removed, always-on tokens vs budget, warnings.

**Generated block format.** Exactly one block per file, bounded by markers; everything outside is user-owned and never modified.

```markdown
<!-- kipi:begin (do not edit; regenerate with `kipi sync`) -->
## Conventions (acme)
<contents of conventions/acme.md>

## Known repos (acme)
- billing-api (billing, invoices) — Invoicing and payment orchestration; OpenAPI in api/openapi.yaml · ~/work/acme/billing-api · notes: ~/work/.kipi-acme/repos/billing-api/notes.md
- ml-platform (ml) — Shared training and feature tooling · ~/work/acme/ml/ml-platform
…

## This repo
Scope: acme-ml. Related: billing-api, ml-platform (read access granted).
For any repo above, read its code at the listed path; check its notes first if listed.
Lookup: `kipi where <alias>`, `kipi grep <term> --repos`.
<!-- kipi:end -->
```

If the file does not exist, sync creates it containing only the block. If the file exists without markers, the block is appended after a blank line.

**Concurrency and safety.** `sync` takes a file lock on `~/.config/kipi/lock`. Writes are atomic (write temp, rename). Symlink targets are absolute paths. Never follow symlinks when deleting. Malformed `settings.local.json` is reported and left untouched.

## 9. Summaries, aliases and notes

The registry is only as good as its summaries, so v1 makes them cheap to get and safe to maintain.

- **Drafting (`scan`).** For a new repo, `scan` drafts `summary` from, in order: the manifest description (`Cargo.toml` `package.description`, `pyproject.toml` `project.description`, `package.json` `description`), then the first prose paragraph of `README.md` (stripped of badges, headings and links, cut to ~140 chars), then nothing. It drafts `aliases` from the repo name (split on `-`/`_`, drop generic words like `api`, `service`, `lib`) and appends a language/key-file hint when obvious (e.g. "OpenAPI in api/openapi.yaml"). Drafted values carry `# kipi:draft`.
- **Supervision.** Drafted values are shown as `draft summary` in `tree` and reported by `doctor` (info). Once a human edits them (by hand or with `kipi edit`) the marker goes away and kipi never rewrites them.
- **Notes.** Optional, hand-written, never generated. `kipi edit notes:<repo>` creates one from a short template.
- **Determinism.** Drafting is deterministic and offline; no LLM in v1. An opt-in `--llm` drafting hook (shelling out to the user's own agent, e.g. `claude -p`) is future work.

## 10. Doctor, budgets and leak checks

`doctor` runs all checks and exits 2 on any error-level finding. `--fix` applies only fixes marked safe.

| Check | Level | Safe fix |
| --- | --- | --- |
| Always-on tokens > `budget_tokens` for a repo (estimated over the rendered block + skill descriptions) | warn | — (prints the largest contributors) |
| Repo's `remote.origin.url` matches a scope's `remote_patterns` but resolved to a different scope | error | — (prints the `scope =` line to add) |
| Repo bound to a scope whose root is not an ancestor and no explicit `scope` set | warn | — |
| `related` or grant crossing unrelated scopes | error | — |
| Alias collision within a registry | error | — |
| Skill referenced but missing in the store | error | — |
| Reference into another store (`extends`, `scope`, `related`, `exclude`, or a qualified skill from a store without `share_skills`) | error | — |
| Repo whose `remote` matches a scope's `remote_patterns` in one store but whose entry lives in another store (e.g. a work repo recorded in the personal store) | error | — (prints the store to move it to) |
| Two checkouts on this machine matching the same entry, or a checkout whose remote differs from its entry's `remote` | warn | — |
| Entry without `remote` whose checkout has one | info | fill `remote` |
| Absolute home path in `bindings.toml`, notes or conventions | error | — |
| Unscoped repo | warn | — |
| Repo with no summary, or a summary still marked draft | info | — |
| Repo in `machine.toml` whose path no longer exists | warn | remove entry |
| Symlink in a repo pointing outside every mounted store or to a missing target | warn | remove |
| Grant entry pointing to a missing path | warn | re-sync repo |
| Link present on disk but absent from manifest (foreign) | info | — |
| Missing `.git/info/exclude` entries | warn | add |
| Generated block in a tracked file | info | — |
| Skill `description` > 1,024 chars or missing | warn | — |
| Notes > 1,500 tokens | info | — |
| Registry > 40 repos (suggest splitting the scope) | info | — |

Token counting uses `tiktoken-rs` (`cl100k_base` default) as an estimate; counts are cached per file by (path, mtime, size) in `~/.cache/kipi/` so `tree` and `status` stay under ~100 ms for 50 repos.

## 11. Multiple machines

kipi has no transport of its own and never requires git. Each store is a plain directory, synced however its trust boundary allows: a personal git remote, a dotfiles repo, Syncthing, employer-approved storage, or not at all. Machine-only state (`machine.toml`, `config.toml`, caches) never lives in any store.

- **Paths differ per machine by design.** Stores hold names, remotes and `~`-relative roots; `machine.toml` holds this machine's absolute paths and root overrides; `scan` rebuilds it by matching `remote` first, so a repo cloned at a different path or under a different directory name is still recognized. Registries, blocks and grants are generated per machine at sync.
- **Second machine:** get each store it should see onto the machine by any means, then `kipi init --store <path>` per store (mounts it), `kipi scan`, `kipi sync --all`. For a store in a git repo, `kipi init --store <path> --clone <git-url>` does the first step.
- **A store in a git repo** (the easy path for a personal store shared by two laptops): `kipi push` commits the store's changes and pushes them; `kipi pull` fast-forwards it and runs `sync --all`. Each store syncs on its own, only with its own remote, through the `git` binary run in that store's directory; nothing moves across stores. Pull never merges: diverged histories stop that store with a hint (`git pull --rebase`). Push refuses a store with doctor errors or no upstream, and never commits `generated/`. No custom transports, lockfile or conflict resolution.
- **Personal laptop vs work laptop:** the work laptop mounts both stores; the personal laptop mounts only the personal store and never holds work summaries, notes or conventions.
- **Losing a store is cheap:** `scan` redrafts every entry from READMEs and manifests; only hand edits, notes and skills need a backup.
- **After a store changes:** `kipi sync --all`.
- **Secrets:** none. kipi never reads or stores credentials; agent config beyond the `additionalDirectories` grant (permissions, MCP configs) is out of scope.

## 12. Architecture

One Cargo workspace, three crates. The core is a library with no terminal or network dependencies so a TUI or editor plugin can reuse it later.

```
kipi/
├── Cargo.toml                  # workspace
├── crates/
│   ├── kipi-core/              # lib: model, config, resolve, plan, apply, doctor, scan
│   │   └── src/
│   │       ├── model.rs        # StoreSet, Store, Scope, RepoEntry, RemoteUrl, Skill, Grant, Machine
│   │       ├── config/         # load/validate each store's bindings.toml, machine.toml, config.toml; cross-store checks (toml_edit + serde)
│   │       ├── frontmatter.rs  # YAML frontmatter parse/serialize for SKILL.md
│   │       ├── resolve.rs      # (stores, machine) -> Plan   [pure, fully unit-tested]
│   │       ├── plan.rs         # Plan, RepoPlan, Action enum (Link, WriteBlock, MergeGrant, Remove, Exclude)
│   │       ├── adapters/       # claude.rs, generic.rs  (trait AgentAdapter)
│   │       ├── render/         # registry.rs, block.rs (templates via minijinja)
│   │       ├── apply.rs        # execute Actions against the FS; atomic writes; file lock
│   │       ├── manifest.rs     # .kipi/manifest.toml
│   │       ├── json_merge.rs   # owned-entry merge into settings.local.json
│   │       ├── scan.rs         # repo discovery + summary/alias drafting
│   │       ├── doctor/         # one module per check; Finding {level, code, repo, message, hint, fix: Option<Action>}
│   │       ├── tokens.rs       # tiktoken-rs wrapper + mtime cache
│   │       ├── search.rs       # grep over store files and local checkouts (ignore + regex), scope-filtered
│   │       ├── git.rs          # thin wrapper over the `git` binary (no libgit2)
│   │       └── error.rs        # thiserror types; every user-facing error carries a hint
│   ├── kipi-cli/               # bin: clap commands, output formatting (tree, tables, json)
│   │   └── src/
│   │       ├── main.rs
│   │       ├── commands/       # one file per subcommand
│   │       └── output/         # tree.rs (box drawing + ASCII fallback), json.rs, color.rs
│   └── kipi-test-support/      # fixtures: temp stores, fake repos, snapshot helpers
├── templates/                  # SKILL.md, notes.md, bindings.toml skeletons (embedded via include_str!)
├── docs/                       # mdBook: concepts, config reference, adapters, doctor codes, decisions
├── examples/
│   ├── acme-store/             # example work store: scopes acme + acme-ml, 6 repos, 4 skills
│   └── personal-store/         # example personal store: scope personal, 4 repos, 2 skills, share_skills = true
└── CLAUDE.md                   # build conventions for Claude Code (§14)
```

**Key traits**

```rust
pub trait AgentAdapter {
    fn id(&self) -> &'static str;                       // "claude" | "generic"
    fn skills_dir(&self, repo: &Path) -> PathBuf;
    fn always_on_file(&self, repo: &Path) -> PathBuf;   // CLAUDE.md / AGENTS.md
    fn render_block(&self, block: &RenderedBlock) -> String;
    fn render_skill_entry(&self, skill: &Skill, target: &Path) -> Action;
    fn render_grant(&self, repo: &Path, grant: &Grant) -> Option<Action>;  // None = agent has no grant mechanism
}
```

**Dependencies (pinned, minimal):** `clap` (derive) + `clap_complete`, `serde`, `serde_json`, `toml_edit`, `serde_yaml` (or `yaml-rust2`), `minijinja`, `thiserror`, `anyhow` (cli only), `ignore`, `regex`, `walkdir`, `tiktoken-rs`, `owo-colors`, `fs4` or `fd-lock`, `tempfile`, `sha2`, `dirs`, `insta` (dev), `assert_cmd` + `predicates` (dev), `proptest` (dev). No tokio, no reqwest, no libgit2, no SQLite.

**Data flow:** `config::load` → `resolve::resolve` → `Plan` → (`apply::run` | `output::tree` | `doctor::run` | `commands::explain`). Every command after load is a function of `Plan`; nothing reads config twice.

## 13. Testing strategy

Test the pure core exhaustively, the filesystem layer with real temp dirs, the CLI with snapshots.

| Layer | Method | Target |
| --- | --- | --- |
| `resolve` | Unit tests on in-memory values; property tests (proptest) that scope resolution is deterministic, `exclude` always wins, and no skill, registry line or grant crosses scopes or stores (except explicitly shared skills) | 100% branch coverage of resolve.rs |
| Config | Fixture TOML files, including invalid ones; assert error messages with `insta` snapshots | Every schema key has a positive and a negative case |
| Scan / drafting | Fixture repos with assorted READMEs and manifests; snapshot drafted entries | Deterministic; never overwrites reviewed values |
| Adapters | Golden files: given a `RepoPlan`, each adapter's rendered block, action list and grant are snapshotted | One snapshot per adapter per fixture repo |
| `apply` | Integration tests in `tempfile::tempdir()`; sync twice is a no-op; removing a binding cleans up; foreign files and user JSON keys untouched | Idempotence, cleanup, non-destruction |
| Doctor | One fixture store per check that must trigger exactly that finding | One test per check code |
| CLI | `assert_cmd` end-to-end on `examples/acme-store` + `examples/personal-store` mounted together: `init`, `scan`, `sync`, `tree --json`, `doctor` exit codes | Every subcommand smoke-tested |
| Leak guard | A test greps the whole repo (code, docs, fixtures) for a configurable denylist of organization names and home paths; CI fails on any hit | Zero hits |

CI: GitHub Actions matrix (ubuntu-latest, macos-latest), `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`, `cargo deny` for licenses and banned crates; release builds via `cargo-dist` producing static binaries and a Homebrew tap formula.

## 14. Build stages for Claude Code

Each stage is a shippable, tagged milestone. Do not start a stage before the previous one's acceptance criteria pass in CI. Write `CLAUDE.md` for the kipi repo itself first; it is the contract Claude Code works under.

**Repo `CLAUDE.md` (initial content)**

```markdown
# kipi
Rust workspace: kipi-core (lib, no I/O beyond fs), kipi-cli (bin), kipi-test-support.
Spec: roadmap.md. Rules: no network crates; no SQLite; adapters < 100 lines each; every user-facing error has a hint.
Never hardcode a home path; use dirs + machine.toml. No organization names anywhere (CI denylist).
Tests: unit for resolve, tempdir integration for apply, insta snapshots for rendering and CLI.
Run `cargo test && cargo clippy -D warnings` before claiming a task is done.
Example stores live in examples/acme-store and examples/personal-store and must stay valid.
```

| Stage | Deliverable | Acceptance |
| --- | --- | --- |
| 0 — Scaffold | Workspace, crates, CI, `CLAUDE.md`, example work + personal stores, denylist test | CI green on empty crates |
| 1 — Model + resolve | Model, config loading with validation, multiple stores + repo identity, `resolve.rs` (scopes, skills, registry, related, grants), `kipi resolve --json`, `kipi explain` | Resolve tests pass on both example stores mounted together and on the personal store alone; `explain` output matches §6 for every example repo |
| 2 — Sync | `scan` with drafting, rendering, adapters (claude, generic), apply + manifest, read grants, `sync`, `status`, authoring commands | Zero to synced on the example repos with `init`, `scan`, `sync --all`; sync twice is a no-op; unbinding cleans up links and grants |
| 3 — Visibility + doctor | `tree`, `where`, `grep`, `edit`, token counting with cache, all §10 checks, `--fix` | `tree` matches the §7 sketch; < 100 ms for 50 repos; one test per doctor check; `doctor` on the example store exits 0 |
| 4 — Release 0.1.0 | `cargo-dist` release, completions, docs, README with GIF, final leak audit (Homebrew tap deferred past 0.1.0, see `docs/decisions.md`) | shell installer → `init`, `scan`, `sync --all` works on a fresh macOS and Linux machine |

Suggested first prompt for Claude Code: "Read roadmap.md and CLAUDE.md. Implement issue #1."

## 15. Future work and open decisions

**Deferred, with the seam already in place**

- Repo cards (`CARD.md` / `CARD.generated.md`) and interface generators (openapi, cargo, python, dbt, sql) with gen/diff/accept supervision. Revisit only if reading the real repo proves insufficient (e.g. repos not cloned locally, very large repos).
- Kiro adapter; `copy` mode for sandboxes that block reads outside cwd; `--commit` mode for teammates without kipi.
- A lockfile, custom store transports (`pull_cmd`/`push_cmd`), auto-pull on sync and conflict resolution (git-based `pull`/`push`/`init --clone` shipped; §11).
- Qualified sharing of anything other than skills across stores (e.g. a personal convention reused at work).
- Mention log: an opt-in record of which registry entries agents resolve, feeding a `doctor` "promote to related" suggestion.
- `--llm` summary drafting through the user's own agent.
- TUI (ratatui) over `kipi-core`'s `Plan`, writing through `toml_edit` only.
- Retrieval server (MCP): `search.rs` and `where` already expose the operations a server would need.
- Windows support (junctions instead of symlinks).

**Open decisions to settle in Stage 1–2**

All five are settled; each links to its entry in `docs/decisions.md`.

1. Registry rendering when a repo is not cloned on this machine. Recommendation: list it with `local: (not cloned here)`. **Decided:** listed, and placed in its scope by `remote_patterns` (§6 rule 3). See `docs/decisions.md`, "Stage 1: resolution engine (#3)".
2. Should `sync` write `AGENTS.md` by default? Recommendation: write it when the generic adapter is enabled (default) or the file already exists. **Decided:** written whenever the generic adapter is enabled. See `docs/decisions.md`, "Stage 2: adapters (#6)".
3. Tracked `CLAUDE.md`: append the block anyway, or write it to an untracked side file the agent also loads? Recommendation: decide after checking which local/untracked memory files each agent currently loads; default to never dirtying a tracked file without `--allow-tracked`. **Decided:** never dirty a tracked file without `--allow-tracked`; Claude Code gets `CLAUDE.local.md`. See `docs/decisions.md`, "Stage 2: adapters (#6)".
4. Default `grant`: `related` (minimal) vs `registry` (frictionless). Recommendation: `related`; document `registry` for users who want no prompts at all. **Decided:** `related`. See `docs/decisions.md`, "Stage 1: resolution engine (#3)".
5. Whether `conventions/<scope>.md` may use `@import`-style includes. Recommendation: no; keep it a flat file so every adapter renders identically. **Decided:** no. See `docs/decisions.md`, "Stage 1: resolution engine (#3)".

## 16. Glossary

| Term | Meaning |
| --- | --- |
| Store | A directory holding skills, notes, conventions and `bindings.toml`; one per trust boundary; several may be mounted |
| Remote | A repo's normalized `remote.origin.url`; its identity across machines |
| Scope | Visibility boundary; owns skills and a registry; may extend a parent |
| Repo entry | A repo's summary, aliases, related repos and skill overrides in `bindings.toml` |
| Registry | Generated always-on list of repos in a scope with aliases, summaries and paths |
| Related | A declared relation that lists the repo in the this-repo section and grants read access |
| Grant | The directories an agent may read without prompting, written by an adapter |
| Draft | A summary or alias list written by `scan` and not yet reviewed by a human |
| Notes | Optional hand-written `notes.md` per repo |
| Adapter | Code that maps a `Plan` onto one agent's file conventions |
| Generated block | The marker-delimited section kipi owns inside CLAUDE.md / AGENTS.md |
| Manifest | `.kipi/manifest.toml`; everything sync wrote into a repo, for clean removal |
| Always-on | Text loaded on every agent turn; budgeted per repo |
