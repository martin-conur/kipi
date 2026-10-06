# `--json` output

`kipi resolve --json` prints the resolved plan; `kipi resolve <repo> --json` and
`kipi explain <repo> --json` print the same document restricted to one repo and the registry it
renders; `kipi scan --json`, `kipi sync --json` and `kipi status --json` print their own reports
([Scan](#scan), [Sync](#sync), [Status](#status)). The shape is stable: fields
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
| `registry` | Decided<string> \| null | Scope whose registry the repo renders |
| `related` | [Related] | `related` repos, in declared order |
| `grant` | Grant | The read grant |
| `agents` | Decided<[string]> | Adapters to render (`claude`, `generic`) |
| `budget_tokens` | Decided<number> | Always-on token budget |

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
scopes' roots at the same depth, or without a free name), `vanished` (a `machine.toml` path that no
longer exists).

## Sync

`kipi sync --json` (with or without `--dry-run`) prints what changed (or would change).

| Field | Type | Meaning |
| --- | --- | --- |
| `version` | number | Schema version, currently `1` |
| `dry_run` | bool | `true` when nothing was written |
| `repos` | [SyncRepo] | The named repos in the order given, or with `--all` every repo entry sorted by name, then orphaned checkouts sorted by entry name |
| `registries` | [{`store`, `report`: Report}] | One per mounted store: its `generated/` registries |

**SyncRepo**: `name`, `store` (string \| null; `null` for an orphan), `path` (the checkout from
`machine.toml`, or `null`), `status`, `always_on` (AlwaysOn \| null; `null` for an orphan),
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

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | User error: bad arguments, an invalid or inconsistent store, an unknown repo, a command not implemented yet |
| 2 | `doctor` found an error-level problem |
| 3 | I/O failure reading or writing a file |
