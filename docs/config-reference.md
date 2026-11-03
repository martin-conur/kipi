# Configuration reference

kipi reads three TOML files. All three are validated on every load: an unknown key is an error
naming the closest valid one, and every error points at `file:line` with a hint. kipi writes them
back with comment-preserving round-trips, so hand-edits and comments survive.

| File | Where | What |
| --- | --- | --- |
| `bindings.toml` | The root of each store | The store's scopes and repo entries (portable) |
| `machine.toml` | `~/.config/kipi/` | Where each repo is cloned on this machine (never in a store) |
| `config.toml` | `~/.config/kipi/` | Mounted stores and preferences |

`$KIPI_CONFIG_DIR` replaces `~/.config/kipi`, `$KIPI_CACHE_DIR` replaces `~/.cache/kipi`, and
`$KIPI_STORE` (one path, or several separated by `:`) replaces the configured store list.

In the key column, `<scope>` and `<repo>` stand for a name you choose. Names use letters, digits,
`-`, `_` and `.`, starting with a letter or digit. Paths may start with `~/`.

This page is checked by a test (`crates/kipi-core/tests/config_reference.rs`): every key the
loaders accept must have a row here, and every row must be a key they accept.

## bindings.toml

One per store. Scope and repo names are unique across all mounted stores. `kipi add scope` (or
`kipi init --scope`) writes a `[scopes.<scope>]` table with `extends`, `roots`,
`remote_patterns` (`add scope --remote-pattern`), `skills` and `registry` (on unless
`--no-registry`, or a scope it extends already has one); `kipi scope` changes `registry`, `roots`,
`remote_patterns`, `extends`, `skills`, `always` and `attributes` of an existing one;
`kipi bind` / `kipi unbind` change a repo entry's lists and `attributes`. `kipi add repo`
and `kipi scan` write `[repos.<name>]` tables; `kipi remove repo` deletes one.

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `store.name` | string | required | The store's name; unique among mounted stores; used in `<store>/<skill>` and in output |
| `store.version` | integer | required | Schema version; this kipi reads `1` |
| `store.share_skills` | bool | `false` | Let other mounted stores reference this store's skills as `<store>/<skill>` |
| `defaults.budget_tokens` | positive integer | `4000` | Always-on token cap per repo (estimated) |
| `defaults.agents` | array of `"claude"`, `"generic"` | `["claude", "generic"]` | Adapters to render |
| `defaults.grant` | `"related"`, `"registry"` or `"none"` | `"related"` | Which directories the agent may read without prompting (see [Concepts](concepts.md#grants)) |
| `scopes.<scope>.extends` | string | none | Parent scope, in the same store; skills and conventions are inherited top-most first |
| `scopes.<scope>.roots` | array of paths | `[]` | Directories whose repos belong to the scope; the deepest matching root wins. `machine.toml` `roots` overrides them per machine |
| `scopes.<scope>.skills` | array of skill refs | `[]` | Skills every repo in the scope gets: a bare name from this store, or `<store>/<skill>` from a store that shares its skills |
| `scopes.<scope>.always` | array of paths | `[]` | Store-relative Markdown files (usually `conventions/<scope>.md`) rendered into the generated block |
| `scopes.<scope>.registry` | bool | `false` | Build a registry for this scope (its repos and its descendants') and render it into the block |
| `scopes.<scope>.remote_patterns` | array of strings | `[]` | Remote prefixes (`github.com/acme-org/`) that mark a repo as belonging here; `doctor` reports repos that match but resolved elsewhere |
| `scopes.<scope>.agents` | array | inherited | Overrides `defaults.agents` for the scope and its descendants |
| `scopes.<scope>.grant` | string | inherited | Overrides `defaults.grant` |
| `scopes.<scope>.budget_tokens` | positive integer | inherited | Overrides `defaults.budget_tokens` |
| `scopes.<scope>.attributes` | table of strings | `{}` | Free-form `key = "value"` settings every repo in the scope and its descendants inherits; a nearer scope's value wins. See `repos.<repo>.attributes` |
| `scopes.<scope>.on_sync` | array of tables | `[]` | Sync commands every repo in the scope and its descendants runs on `kipi sync` (`[[scopes.<scope>.on_sync]]`). See `repos.<repo>.on_sync` |
| `scopes.<scope>.on_sync[].name` | string | required | The command's name; unique within one table's `on_sync`. A descendant scope's or a repo's command of the same name replaces it |
| `scopes.<scope>.on_sync[].run` | string or array of strings | required | The program and its arguments. A string is split into words without a shell (quotes group, `\` escapes; `$VAR`, `~`, globs, `;` and `|` are plain text); an array is taken word for word. `{attr.<key>}` fills in the repo's attribute inside its own word (`{{`, `}}` for literal braces); the program word takes no placeholder. Want a shell? `run = ["sh", "-c", "…"]`; pass attributes to it as positional arguments (`"$1"`), never inside the script, since approval covers the template, not the values |
| `scopes.<scope>.on_sync[].when` | string or array of strings | none | Attribute keys (an `attr.` prefix is allowed) that must all be set; otherwise the command is skipped silently |
| `scopes.<scope>.on_sync[].worktrees` | bool | `false` | Also run in the checkout's linked worktrees (`kipi sync --hook` included) |
| `repos.<repo>.remote` | string | none | Normalized origin URL (`github.com/acme-org/billing-api`); filled by `scan`; the primary match key across machines |
| `repos.<repo>.summary` | string | none | One line: what the repo is and where to look. A trailing `# kipi:draft` marks it drafted |
| `repos.<repo>.aliases` | array of strings | `[]` | Other names for the repo in the registry; matched exactly after case-folding |
| `repos.<repo>.related` | array of repo names or `{ repo, via }` tables | `[]` | Repos listed in this repo's section and granted for reads; must be in the same scope chain. Strings and tables can be mixed |
| `repos.<repo>.related[].repo` | string | required | In a table item: the related repo entry name |
| `repos.<repo>.related[].via` | string | none | In a table item: how this repo connects to it, shown next to it in the block. One line, at most 120 characters; one direction only (`b`'s own item to `a` has its own `via`) |
| `repos.<repo>.skills` | array of skill refs | `[]` | Skills added to the scope's |
| `repos.<repo>.exclude` | array of skill refs | `[]` | Skills removed from the effective set; always wins |
| `repos.<repo>.scope` | string | from path | Explicit scope; beats root membership |
| `repos.<repo>.agents` | array | inherited | Overrides the scope's `agents` |
| `repos.<repo>.grant` | string | inherited | Overrides the scope's `grant` |
| `repos.<repo>.budget_tokens` | positive integer | inherited | Overrides the scope's `budget_tokens` |
| `repos.<repo>.attributes` | table of strings | `{}` | Free-form per-repo settings for the tools kipi syncs into it (`"tracker.project" = "PROJ"`); kipi does not interpret them. Keys are dotted segments of letters, digits, `-` and `_` (quoted, nested and sub-table spellings are the same key); values are one-line strings. The repo's value beats its scope chain's. Read with `kipi where` / `kipi resolve --json`, queried with `kipi where --attr key=value` |
| `repos.<repo>.on_sync` | array of tables | `[]` | Sync commands: programs `kipi sync` runs in the checkout after materializing it, when due (never run here, argv changed, or last run failed) and approved on this machine (`kipi sync --allow-commands`). Added after the scope chain's; one with a scope command's `name` replaces it. `kipi resolve` shows what would run |
| `repos.<repo>.on_sync[].name` | string | required | As `scopes.<scope>.on_sync[].name` |
| `repos.<repo>.on_sync[].run` | string or array of strings | required | As `scopes.<scope>.on_sync[].run` |
| `repos.<repo>.on_sync[].when` | string or array of strings | none | As `scopes.<scope>.on_sync[].when` |
| `repos.<repo>.on_sync[].worktrees` | bool | `false` | As `scopes.<scope>.on_sync[].worktrees` |

Precedence for `agents`, `grant` and `budget_tokens`: repo > scope (walking up `extends`) >
`defaults`. For each key of `attributes`: repo > scope (walking up `extends`); there are no store
defaults. For `on_sync`: the chain's commands top-most scope first, then the repo's own; a later
command with an earlier one's `name` replaces it in place. A command whose placeholder names an
unset attribute is skipped with a warning; add `when` to skip it silently instead.

```toml
[store]
name = "acme"
version = 1

[scopes.acme]
roots = ["~/work/acme"]
skills = ["code-review", "personal/commit-style"]
always = ["conventions/acme.md"]
registry = true
remote_patterns = ["github.com/acme-org/"]

[repos.billing-api]
remote = "github.com/acme-org/billing-api"
summary = "Invoicing and payment orchestration; OpenAPI in api/openapi.yaml"
aliases = ["billing", "invoices"]
attributes = { "tracker.project" = "BILL", "tracker.board" = "Payments" }

# Run an installer in every acme repo that has a tracker project (needs approving once
# per machine: `kipi sync --allow-commands`).
[[scopes.acme.on_sync]]
name = "agent-workflow"
run = "some-installer --local-only --key {attr.tracker.project} --board {attr.tracker.board}"
when = "tracker.project"
```

## machine.toml

Written by `kipi scan` and `kipi add repo`; you rarely edit it. Paths are absolute (or `~/`).

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `hostname` | string | none | A label for this machine |
| `roots.<scope>` | array of paths | the scope's `roots` | Replaces a scope's roots on this machine |
| `repos.<repo>` | path | absent | Where repo entry `<repo>` is cloned here; absent means not on this machine |
| `approved_commands` | array of strings | `[]` | Sync commands (`on_sync`) allowed to run on this machine, as `sha256:` keys over the store, the declaring table, the name, `worktrees` and the unfilled `run`. Written by `kipi sync --allow-commands` (or a yes at its prompt); a changed `run` needs approving again, a changed attribute value does not |

```toml
hostname = "laptop-a"

[roots]
"acme" = ["/Volumes/work/acme"]

[repos]
"billing-api" = "/Volumes/work/acme/billing-api"
```

## config.toml

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `stores` | array of paths | `["~/.agents/store"]` | The mounted stores. `kipi init` and `kipi stores --unmount` maintain it |
| `editor` | string | `$EDITOR` | The command `kipi edit` runs when `$EDITOR` is unset |
| `tokenizer` | string | `"cl100k_base"` | Encoding for token estimates: `cl100k_base`, `o200k_base`, `p50k_base`, `p50k_edit` or `r50k_base` |

Store precedence: `--store` flags > `$KIPI_STORE` > `stores` > `~/.agents/store`; each source
replaces the ones below it as a whole.

```toml
stores = ["~/.agents/store", "~/work/.kipi-acme"]
editor = "hx"
tokenizer = "cl100k_base"
```

## SKILL.md frontmatter

Not TOML, but configuration all the same: standard Agent Skills fields plus kipi's own under
`x-kipi`, so the file stays valid for every agent.

```yaml
---
name: dbt-conventions          # equals the directory name
description: Use when writing or reviewing dbt models, tests or sources.   # ≤ 1,024 chars
x-kipi:
  scope: acme
  tags: [data, sql]
---
```
