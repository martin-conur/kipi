# `--json` output

`kipi resolve --json` prints the resolved plan; `kipi resolve <repo> --json` and
`kipi explain <repo> --json` print the same document restricted to one repo and the registry it
renders; `kipi scan --json`, `kipi sync --json` and `kipi status --json` print their own reports
([Scan](#scan), [Sync](#sync), [Status](#status)), as do `kipi stores --json` ([Stores](#stores)),
the authoring commands ([Authoring](#authoring)), `kipi where --json` ([Where](#where)) and
`kipi grep --json` ([Grep](#grep)) and `kipi tree --json` ([Tree](#tree)); `kipi doctor --json` prints its findings
(shape in [docs/doctor.md](doctor.md#json)). The shape is stable: fields
are only ever added, and anything that renames, removes or changes the meaning of a field bumps
`version`. Output is pretty-printed, deterministic (two runs
over the same stores and `machine.toml` are byte-identical) and ends with a newline.

```bash
kipi resolve --json | jq -r '.repos[] | "\(.name)\t\(.scope.value)"'
kipi explain events-pipeline --json | jq '.repos[0].grant.dirs[].path'
```

Paths are absolute, as on this machine. Errors are never JSON: they go to stderr as
`error: …` / `hint: …` with a non-zero exit code (see [Exit codes](#exit-codes)).

## Document

| Field | Type | Meaning |
| --- | --- | --- |
| `version` | number | Schema version, currently `1` |
| `repos` | [Repo] | One per repo entry of every mounted store, sorted by name |
| `registries` | [Registry] | One per scope with `registry = true`, sorted by scope |

## Shared shapes

**Source**: why a decision was made.

| Field | Type | Meaning |
| --- | --- | --- |
| `rule` | string | The key or rule that decided it: `scopes.acme.skills`, `machine.toml [roots] acme`, `built-in default grant` |
| `at` | string \| null | Where it is declared: `store:path-in-store:line` (`acme:bindings.toml:13`), or `path:line` for a file outside every store (`machine.toml`); `null` for a built-in default |

Lines point at the `[scopes.<name>]` / `[repos.<name>]` table header that holds the key.

**Decided**: `{ "value": …, "source": Source }`.

## Repo

| Field | Type | Meaning |
| --- | --- | --- |
| `name` | string | Entry name |
| `store` | string | Store owning the entry |
| `at` | string | Its `[repos.<name>]` header, as in Source `at` |
| `path` | string \| null | Checkout on this machine; `null` when not cloned here |
| `scope` | Decided<string \| null> | Resolved scope; `null` when unscoped |
| `chain` | [string] | The scope's `extends` chain, top-most first, ending with the scope; `[]` when unscoped |
| `skills` | [Skill] | Effective skills: the chain top-down, then the repo's own, minus `exclude` |
| `excluded` | [Skill] | The repo's `exclude` entries |
| `always` | [AlwaysOn] | Always-on content in render order |
| `registry` | `Decided<string>` \| null | Scope whose registry the repo renders |
| `related` | [Related] | `related` repos, in declared order |
| `grant` | Grant | The read grant |
| `agents` | Decided<[string]> | Adapters to render (`claude`, `generic`) |
| `budget_tokens` | `Decided<number>` | Always-on token budget |

**Skill**: `name` (directory name), `store` (store holding it), `reference` (as rendered:
`personal/commit-style` for a shared skill), `dir` (absolute), `inherited_from` (the scope whose
`skills` bound it, or `null` when the repo's own `skills` / `exclude` named it), `source`.

**AlwaysOn**, tagged by `kind`:
- `{"kind": "file", "path", "scope", "source"}`: a scope's `always` file.
- `{"kind": "registry", "scope", "source"}`: the registry of `scope`.
- `{"kind": "this-repo"}`: the this-repo section.

**Related**: `name`, `scope` (string \| null), `path` (checkout or `null`), `notes`
(`repos/<name>/notes.md` in the store, or `null`), `source`.

**Grant**: `mode` (Decided<`"related"` \| `"registry"` \| `"none"`>) and `dirs`, each
`{ "path", "kind", "store", "repo", "source" }` where `kind` is `store-repos` (the owning store's
`repos/`; `store` set), `store-skills` (a store's `skills/`; `store` set) or `repo` (a checkout;
`repo` set). The field that does not apply is `null`.

## Registry

| Field | Type | Meaning |
| --- | --- | --- |
| `scope` | string | The scope with `registry = true` |
| `store` | string | Store owning it |
| `scopes` | [string] | The scope and its descendants, sorted |
| `entries` | [Entry] | Sorted by name |
| `collisions` | [{`key`, `repos`}] | Alias keys claimed by more than one entry |
| `source` | Source | The `registry = true` scope |

**Entry**: `name`, `aliases` (as written), `aliases_draft`, `summary` (string \| null),
`summary_draft`, `scope`, `local` (checkout or `null` = not cloned here), `notes`, `keys` (lookup
keys: name and aliases, case-folded and whitespace-normalized), `source`.

## Scan

`kipi scan --json` (with or without `--dry-run`) prints a report instead of the plan. Same
rules: `version` is shared with the plan document, paths are absolute, lists are deterministic.

| Field | Type | Meaning |
| --- | --- | --- |
| `version` | number | Schema version, currently `1` |
| `dry_run` | bool | `true` when nothing was written |
| `roots` | [string] | Directories walked (scope roots after `machine.toml [roots]`, or the given paths) |
| `missing_roots` | [string] | Roots or paths that do not exist here |
| `checkouts` | [string] | Every git checkout found, sorted |
| `added` | [Added] | New `[repos.<name>]` entries |
| `recorded` | [{`entry`, `path`, `previous`}] | `machine.toml [repos]` lines written; `previous` is the vanished path a moved checkout replaced, else `null` |
| `remotes_filled` | [{`entry`, `store`, `remote`}] | `remote` filled on entries that lacked one |
| `refreshed` | [{`entry`, `store`, `field`, `value`}] | Drafts rewritten by `--refresh-drafts`; `field` is `summary` or `aliases`, `value` as written in TOML |
| `warnings` | [Warning] | Things scan did not act on |
| `not_found` | [string] | Entries of the mounted stores with no checkout on this machine, sorted |
| `files` | [string] | Files written (or, with `dry_run`, that would be) |

**Added**: `name`, `store`, `scope`, `explicit_scope` (`scope =` was written because the checkout
lies outside every root), `path`, `remote` (string \| null), `summary` (string \| null; a draft),
`aliases` ([string]; drafts).

**Warning**: `code`, `entry` (string \| null), `path`, `message`, `hint`. Codes:
`scope-conflict` (the remote matches another scope's `remote_patterns`), `remote-differs` (a
recorded checkout's origin differs from its entry's `remote`), `duplicate-checkouts` (several
checkouts match one entry, or clone one new repo; `entry` is `null` then), `ambiguous-match`
(several entries match a checkout), `unplaced` (a new checkout outside every root, under two
scopes' roots at the same depth, or without a free name), `foreign-remote` (a new checkout placed
in one store whose remote matches another store's `remote_patterns`; nothing was added, `entry`
is `null`), `vanished` (a `machine.toml` path that no
longer exists).

## Sync

`kipi sync --json` (with or without `--dry-run`) prints what changed (or would change).

| Field | Type | Meaning |
| --- | --- | --- |
| `version` | number | Schema version, currently `1` |
| `dry_run` | bool | `true` when nothing was written |
| `repos` | [SyncRepo] | The named repos in the order given, or with `--all` every repo entry sorted by name, then orphaned checkouts sorted by entry name; each checkout is followed by its linked worktrees. With no repo named, just the working tree `kipi sync` ran in |
| `registries` | [{`store`, `report`: Report}] | One per mounted store: its `generated/` registries |

**SyncRepo**: `name`, `store` (string \| null; `null` for an orphan), `path` (the checkout from
`machine.toml`, the linked worktree, or `null`), `worktree` (bool: `path` is a linked worktree of the
entry's checkout), `status`, `always_on` (AlwaysOn \| null; `null` for an orphan),
`notices` ([{`message`, `hint`}]: what an adapter declined to do, such as a block not written into a
tracked file), `report` (Report \| null; `null` when the repo was skipped). `status` is one of
`changed`, `unchanged`, `not_cloned` (no checkout in `machine.toml`; skipped), `checkout_missing`
(the recorded checkout is gone; skipped), `orphan` (a checkout `machine.toml` lists for an entry no
mounted store has, synced only with `--all`: everything its manifest recorded was removed).

**AlwaysOn**: `tokens` (estimate over the generated block plus the descriptions of the repo's
skills; characters ÷ 4 until the tokenizer lands), `budget` (`budget_tokens`), `over` (bool). It is
the startup cost to one agent session, so it does not depend on which adapters write files: every
adapter writes the same block and links the same skills, and `--agents` leaves it unchanged.

**Report**: `changes` ([Change], in the order they were made), `links` (`{added, removed}`),
`blocks` (`{written, removed}`: generated blocks, or registry files in a store), `grants`
([{`path`, `added`, `removed`}]: read-grant entries per settings file), `exclude_changed` (bool),
`skipped` ([{`path`, `message`, `hint`}]: actions not carried out, such as a file in the way
without `--force`), `foreign` ([string]: entries beside kipi's links that kipi did not write).

**Change**: `op` (`create`, `update`, `replace`, `remove`), `what` (`link`, `block`, `file`, `dir`,
`exclude`, `grant`), `path`.

## Status

`kipi status --json` and `kipi status <repo> --json`. Nothing is written: the state comes from a
dry-run sync.

| Field | Type | Meaning |
| --- | --- | --- |
| `version` | number | Schema version, currently `1` |
| `repos` | [StatusRepo] | One per repo entry sorted by name, or only the named one |
| `draft_summaries` | number | How many of those repos have a summary still marked `# kipi:draft` |

**StatusRepo**: `name`, `store`, `scope` (string \| null when unscoped), `path` (string \| null),
`state`, `pending` ([Change] a sync would make, or `null` when the repo is not cloned here),
`always_on` (AlwaysOn), `draft_summary` (bool). `state` is one of `synced` (disk and manifest match
the plan), `stale` (synced before; a sync would change something), `missing` (cloned here, never
synced), `not_cloned`, `checkout_missing`.

## Stores

`kipi stores --json` lists the mounted stores, in mount order.

| Field | Type | Meaning |
| --- | --- | --- |
| `version` | number | Schema version, currently `1` |
| `stores` | [Store] | One per mounted store |

**Store**: `name`, `path`, `git` (`{clean, remote}` \| null: `clean` when nothing under the store
is uncommitted, `remote` when its repository has a remote; `null` when the store is not in a git
work tree), `share_skills` (bool), `scopes` ([string], file order), `repos` (number of entries).

## Authoring

`kipi init`, `kipi stores --unmount`, `kipi add skill`, `kipi add repo`, `kipi add scope`, `kipi bind` and
`kipi unbind` with `--json` (with or without `--dry-run`) print what they changed (or would).

| Field | Type | Meaning |
| --- | --- | --- |
| `version` | number | Schema version, currently `1` |
| `dry_run` | bool | `true` when nothing was written |
| `command` | string | `init`, `unmount`, `add skill`, `add repo`, `add scope`, `bind` or `unbind` |
| `store` | string \| null | The store the command created, mounted, unmounted or edited |
| `changes` | [Edit] | What changed, in order |
| `files` | [string] | Directories created, the skill directory installed, files written and the link-back symlink (or, with `dry_run`, that would be) |
| `git_init` | string \| null | The directory `init --git` ran (or would run) `git init` in |

**Edit**: `op` (`add`, `remove`, `keep`: already as asked, nothing changed), `what` (the list or
thing changed: `skills`, `related`, `exclude`, `aliases`, `scopes.<scope>.skills`, `store`,
`store <name>` (a new `[scopes.<scope>]` or `[repos.<name>]` table), `extends`, `roots`, `registry`,
`config.toml stores`, `machine.toml`, ...), `value`, `note` (string \| null: why nothing changed,
or what follows).

## Where

`kipi where <alias> --json`: the one repo entry the alias names (an ambiguous or unknown alias is an
error on stderr, exit 1).

| Field | Type | Meaning |
| --- | --- | --- |
| `version` | number | Schema version, currently `1` |
| `name` | string | Entry name |
| `store` | string | Store owning the entry |
| `scope` | string \| null | Resolved scope; `null` when unscoped |
| `aliases` | [string] | `aliases`, as written |
| `aliases_draft` | bool | The alias list is still marked `# kipi:draft` |
| `summary` | string \| null | `summary` |
| `summary_draft` | bool | The summary is still marked `# kipi:draft` |
| `local` | string \| null | Checkout on this machine; `null` when not cloned here |
| `notes` | string \| null | `repos/<name>/notes.md` in the store, when it exists |
| `notes_text` | string | The notes' contents; only with `--notes` |

## Grep

`kipi grep <pattern> --json`.

| Field | Type | Meaning |
| --- | --- | --- |
| `version` | number | Schema version, currently `1` |
| `searched` | string | What was searched: `the scope chain of acme-ml`, `the unscoped repos of store personal`, `every mounted store` |
| `hits` | [Hit] | Matching lines: conventions, notes, skills, then checkouts (by repo name); files in path order |
| `truncated` | bool | More lines matched than `--limit` allowed |

**Hit**: `path` (absolute), `line` (1-based), `text` (the line, cut to 300 characters with `…`),
`source`: `{"kind": "conventions", "scope"}`, `{"kind": "notes", "repo"}`,
`{"kind": "skill", "store", "name"}` or `{"kind": "checkout", "repo"}`.

## Tree

`kipi tree --json`, with the same filters as the text (`--repo`, `--skill`, `--store`). The
display flags (`--budget`, `--machine`) change only the text: the JSON always carries every
field. Nothing is written but the caches in `~/.cache/kipi`; sync states come from a dry-run sync.

```bash
kipi tree --json | jq -r '.. | objects | select(.state?) | "\(.name)\t\(.always_on.tokens)"'
kipi tree --skill code-review --json | jq -r '.. | objects | select(.state?) | .name' | fzf
```

| Field | Type | Meaning |
| --- | --- | --- |
| `version` | number | Schema version, currently `1` |
| `tokenizer` | string | The encoding the counts come from (`config.toml` `tokenizer`) |
| `stores` | [TreeStore] | Mounted stores in mount order; with `--repo` / `--skill`, only those with a match |

**TreeStore**: `name`, `path`, `git` (`{clean, remote}` \| null, as in [Stores](#stores)), `shares_skills` (bool), `scopes` ([TreeScope]: scopes without `extends`, in file order),
`unscoped` ([TreeRepo]: repos that resolve to no scope).

**TreeScope**: `name`, `roots` ([string] as declared), `machine_roots` ([string] from
`machine.toml [roots]`, or `null` when not overridden here), `extends` (string \| null),
`always_on_tokens` (number: the chain's `always` files, the registry the scope renders and the
descriptions of the chain's skills, i.e. what every repo in the scope pays before its own section
and skills), `skills` ([string]: the scope's own `skills` as written), `registry` (`{ "repos":
number, "tokens": number }` when the scope has `registry = true`, else `null`), `scopes`
([TreeScope] extending it), `repos` ([TreeRepo] resolved to it, sorted by name). With `--repo` or
`--skill`, scopes with nothing matching under them are left out.

**TreeRepo**: `name`, `path` (string \| null), `state` (as in [Status](#status)), `always_on`
(AlwaysOn), `notes` (path of `repos/<name>/notes.md`, or `null`), `draft_summary` (bool), `skills`
([{`reference`, `mark`}]: effective and excluded skills in the order the scope chain and the repo
bind them; `mark` is `inherited`, `explicit` or `excluded`), `related` ([{`name`, `access`}];
`access` is `granted`, `not_granted` or `not_cloned`).

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | User error: bad arguments, an invalid or inconsistent store, an unknown repo, a command not implemented yet |
| 2 | `doctor` found an error-level problem (its report, JSON included, is still printed to stdout) |
| 3 | I/O failure reading or writing a file |
