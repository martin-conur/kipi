# Concepts

## Registry first

kipi does not copy knowledge between repos. It keeps a **registry**: one line per repo with its
name, aliases, a human-reviewed summary and where it lives on this machine. That line is enough
for an agent to resolve "the billing thing" to `billing-api` at `~/work/acme/billing-api` and go
read the real code, README and specs, which are always current. Deeper knowledge comes from the
repo itself, on demand.

So the summaries matter. `kipi scan` drafts them from manifests and READMEs (deterministically,
offline), marks them `# kipi:draft`, and kipi never rewrites one after a human has edited it.

## Five nouns

Everything in the CLI is an operation on one of these.

| Noun | What it is | Lives in | Reaches the agent via |
| --- | --- | --- | --- |
| **Repo entry** | One repo's record: name, aliases, one-line summary, related repos, scope | `bindings.toml` `[repos.<name>]` | Always-on, as a registry line |
| **Skill** | Procedural "how to do X": a directory with `SKILL.md` (Agent Skills format) and optional `references/`, scripts | `<store>/skills/<name>/` | Index-then-load |
| **Notes** | Optional hand-written `notes.md` per repo: invariants, gotchas, "never call v1" | `<store>/repos/<repo>/notes.md` | On demand; the registry line points at it |
| **Scope** | A named visibility boundary (`personal`, `acme`, `acme-ml`); owns skills, conventions and a registry; may `extends` a parent | `bindings.toml` `[scopes.<name>]` | Decides what is rendered and granted where |
| **Registry** | The generated one-line-per-repo map of a scope | Generated at sync | Always-on |

Around them: a **store** is a plain directory holding one `bindings.toml`, its skills, notes and
conventions — one store per trust boundary (personal, work). A **machine** (`machine.toml`) records
where each repo is cloned here; absolute paths never enter a store.

## Three cost tiers

Context is not free, so kipi puts each kind of knowledge in the cheapest tier that works.

| Tier | Paid | What kipi puts there |
| --- | --- | --- |
| **Always-on** | Every turn | Scope conventions, the registry, the this-repo section — capped per repo by `budget_tokens` (default 4,000, estimated) |
| **Indexed** | ~50–100 tokens per skill at startup | Skill descriptions; the body loads only when used |
| **On-demand** | Nothing until used | Other repos' code, notes, `kipi where` / `kipi grep` |

`kipi tree --budget` and `kipi status` show each repo's always-on tokens against its budget;
`doctor` warns when a repo goes over and names the largest contributors.

## Four channels

An agent running in a repo has exactly four ways to get knowledge. kipi only arranges files behind
them — no daemon, no MCP server.

| Channel | Mechanism | kipi uses it for |
| --- | --- | --- |
| 1. Startup injection | Files the agent loads for a directory (`CLAUDE.md`, `AGENTS.md`) | One generated block: conventions, registry, this-repo section |
| 2. Index-then-load | Skill directories (`.claude/skills/`, `.agents/skills/`) | Symlinks to the store's skills |
| 3. File read by path | The agent's own read tool | Other repos' real code; `notes.md` in the store |
| 4. Shell | The agent runs a command | `kipi where <alias>`, `kipi grep <term> --repos` |

The generated block sits between `<!-- kipi:begin … -->` and `<!-- kipi:end -->` markers; everything
outside it is yours and never touched.

## Grants

Reading another repo by absolute path should not prompt on every file. A **grant** is the set of
directories an agent may read without asking; an adapter writes it where the agent looks (for
Claude Code, `permissions.additionalDirectories` in `.claude/settings.local.json`, merged with
yours). `grant` chooses what goes in:

| `grant` | Granted |
| --- | --- |
| `related` (default) | The owning store's `repos/` and `skills/` (plus `skills/` of a store a shared skill comes from), and the checkouts of the repo's `related` repos |
| `registry` | The store dirs and the checkout of every repo in the registry — no prompts at all |
| `none` | Nothing |

An ad-hoc mention ("make this match billing") still works without a grant: the registry gives the
path, and the agent asks once.

## Worktrees

A linked worktree (`git worktree add`) is the same repo entry as its main checkout, never a
second one: `machine.toml` and the registry keep the main checkout's path, and `kipi scan`
skips worktrees. Sync gives each worktree the repo's plan, with its own `.kipi/manifest.toml`.
All working trees of a repo share one `info/exclude`, so kipi's section there lists what any of
them needs. `kipi hook install` adds a git `post-checkout` hook that runs `kipi sync` in each
new worktree; hooks live in the shared git directory, so one install covers them all.

## Scopes and the leak rule

A repo's scope is its explicit `scope`, else the scope whose root is the deepest ancestor of its
path, else `personal` if that scope exists. Resolution never renders a scope's content, or grants
a path, into a repo outside that scope's chain (the scope, its ancestors and descendants). A
`related` entry across unrelated scopes is a hard error.

Stores add a second wall: everything resolves within the store that owns the repo. The only thing
that crosses is a skill a store explicitly shares (`share_skills = true`, referenced as
`<store>/<skill>`). A personal laptop mounts only the personal store and never holds a work
summary, note or convention; `doctor` flags a work repo recorded in the personal store.

`kipi explain <repo>` prints every one of these decisions with the file and line it came from.
