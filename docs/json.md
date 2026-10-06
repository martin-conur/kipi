# `--json` output

`kipi resolve --json` prints the resolved plan; `kipi resolve <repo> --json` and
`kipi explain <repo> --json` print the same document restricted to one repo and the registry it
renders. The shape is stable: fields are only ever added, and anything that renames, removes or
changes the meaning of a field bumps `version`. Output is pretty-printed, deterministic (two runs
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

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | User error: bad arguments, an invalid or inconsistent store, an unknown repo, a command not implemented yet |
| 2 | `doctor` found an error-level problem |
| 3 | I/O failure reading or writing a file |
