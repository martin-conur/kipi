# Decisions

Choices the spec (`roadmap.md`) leaves open, recorded so later stages do not relitigate them.
The spec wins wherever it is explicit.

## Stage 0

- **YAML crate: `yaml-rust2`.** §12 allows `serde_yaml` or `yaml-rust2`; `serde_yaml` is
  deprecated upstream (`0.9.34+deprecated`), so frontmatter parsing will use `yaml-rust2`.
- **File lock crate: `fs4`.** §12 allows `fs4` or `fd-lock`; `fs4` has the simpler
  lock-a-`File` API for `~/.config/kipi/lock`.
- **Toolchain pinned to 1.92.0, edition 2024, `rust-version = "1.92"`.** One version for local
  and CI runs; bump deliberately in its own PR.
- **Workspace version `0.1.0`.** The Stage 4 release cuts 0.1.0; until then `--version` reports it
  unreleased.
- **Banned crates** (`deny.toml`): `tokio`, `reqwest`, `git2`, `rusqlite` per the issue, plus
  `hyper`, `libgit2-sys` and `libsqlite3-sys` so the bans cannot be sidestepped through a
  lower-level crate.
- **License allowlist** is permissive-only (Apache-2.0, MIT, BSD, ISC, Unicode-3.0, Zlib), verified
  against the whole §12 dependency set, not just the crates Stage 0 uses. The one exception is
  `option-ext` (MPL-2.0, pulled in by `dirs`), allowed per crate rather than allowing MPL-2.0
  globally.
- **Leak guard lives in `kipi-test-support`** (`leak_guard.rs` + `tests/leak_guard.rs`). It walks
  files honoring `.gitignore` and `.git/info/exclude` but not the user's global gitignore, so a
  local run sees the same files as CI. Private patterns come from `KIPI_LEAK_DENYLIST`
  (newline-separated regexes, the CI secret) or `KIPI_LEAK_DENYLIST_FILE`. The committed
  `.leak-denylist` holds only generic patterns (credentials, internal-looking hostnames): a real
  organization name in it would be the leak. Home paths whose user is an obvious placeholder
  (`user`, `you`, `example`, `Shared`, ...) are allowed.
- **Templates are embedded from the repo-root `templates/`** with `include_str!("../../../templates/...")`.
  This works in the workspace but not from a packaged crate; Stage 4 (crates.io / cargo-dist) must
  move them under `crates/kipi-core/` or add them to the package `include`.
- **Example store contents.** Where §4/§5 name repos and skills they are used verbatim. The rest
  were chosen to cover the required cases: acme adds `feature-store` (acme-ml, drafted summary and
  aliases, not cloned in the example `machine.toml`) and `web-dashboard` (no summary); personal
  holds `side-tool` (related to `api`), `api` (directory-name clash with acme's `acme-api`), `blog`
  and `dotfiles` (no summary). `dbt-conventions` keeps `x-kipi.scope: acme` exactly as in §5.4,
  although `scopes.acme-ml` is the scope that binds it (an ancestor-owned skill used by a
  descendant).
- **Example `machine.toml` paths** use `/Volumes/work/...` (as in §5.2) and `/srv/code/...`:
  absolute, as `machine.toml` requires, but never a home directory.
- **Fake repo remotes** in `kipi-test-support` use the SSH form for acme repos
  (`git@github.com:acme-org/billing-api.git`) and HTTPS for personal ones, so Stage 1's remote
  normalization is exercised on both shapes by the same fixtures.
- **Duplicate crate versions are denied** (`multiple-versions = "deny"`). The current lockfile has
  none; a genuine duplicate gets a `skip` entry with a reason rather than a blanket `warn`.
- **CI actions are pinned to full commit SHAs** with a `# vX.Y.Z` comment; bump them deliberately.

## Stage 1: model and config loading (#2)

- **Loading checks shape, resolution checks references.** `config::load_store` validates types,
  unknown keys, the store version, names, path forms and SKILL.md frontmatter. Whether `extends`,
  `scope`, `related`, `skills`, `exclude` and `x-kipi.scope` name something that exists is checked
  by resolution (#3) and the multi-store checks (#24), which see the whole store set.
- **`[defaults]` and each of its keys are optional.** Unset keys fall back to the §5.1 values
  (`budget_tokens = 4000`, `agents = ["claude", "generic"]`, `grant = "related"`) through
  `Defaults::{budget_tokens, agents, grant}`. The model keeps them as `Option` so provenance can
  tell "set in the file" from "built in".
- **Scopes and repos accept `agents`, `grant` and `budget_tokens`.** §6's precedence rule
  (repo > scope chain > defaults, "same precedence for `budget_tokens` and `grant`") implies the
  keys exist at every level, although the §5.1 example shows only the repo-level `agents` and `grant`.
- **The version is checked before the schema.** A store written by a newer kipi reports
  `unsupported store version 2`, not the new keys it introduced. `[store] name` and `version` are
  required; `share_skills` defaults to `false`.
- **Names** (store, scope, repo) are ASCII letters, digits, `-`, `_` and `.`, starting with a letter
  or digit. `/` and `:` are reserved for `<store>/<skill>` references and `store:bindings.toml:12`
  provenance. Skill names follow the Agent Skills rule: lowercase letters, digits and `-`, at most
  64 characters, equal to the directory name.
- **Path forms.** `roots`, `config.toml` `stores` and every `machine.toml` path must be absolute or
  start with `~/`; `~` is expanded at load with the home directory from `Env`. `always` entries must
  be relative paths inside the store (no `..`, not absolute), stored canonically (`./a//b` → `a/b`).
- **`remote` must already be normalized** (`host/owner/name`: no scheme, user, port, `:` or
  `.git`). Normalizing a raw origin URL is #24's `RemoteUrl`; until then a raw URL in
  `bindings.toml` is an error that shows the expected form.
- **Values that would break a registry line are rejected:** an empty summary, alias or remote
  pattern, and a multi-line summary.
- **Draft marker.** A value is a draft when its trailing comment's first word is `kipi:draft`
  (`# kipi:draft`, `# kipi:draft from README`). `# kipi:drafted` is not a marker. The flag is read
  from `toml_edit` decor, so it works for multi-line arrays and dotted keys.
- **Config and cache dirs are `~/.config/kipi` and `~/.cache/kipi` on every platform,** as §4 and
  §10 spell them, not `dirs::config_dir()` (which is `~/Library/Application Support` on macOS).
  `dirs` supplies only the home directory. `KIPI_CONFIG_DIR` and `KIPI_CACHE_DIR` override them.
  An absent `machine.toml` or `config.toml` loads as empty.
- **`KIPI_STORE` is a single path for now** (superseded in #24 below). Splitting it into a list, and repeatable `--store`
  flags, land with multiple stores (#24). `config::store_paths` already returns a list.
- **`tokenizer`** must name an encoding tiktoken-rs ships (`cl100k_base`, `o200k_base`,
  `p50k_base`, `p50k_edit`, `r50k_base`).
- **SKILL.md frontmatter must be valid YAML.** The example skills' descriptions contained an
  unquoted `: `, which strict YAML parsers (and Agent Skills validators) reject; they are now quoted.
  `description` is optional at load, because doctor reports a missing one (§10). Standard fields
  kipi does not use (`license`, `allowed-tools`, ...) are kept as they are; only `x-kipi` rejects
  unknown keys. An untouched file renders back byte for byte; changing a field re-emits the YAML in
  its original key order.
- **`skills/` entries:** a directory without `SKILL.md` is an error; hidden entries and plain
  files are skipped.
- **Unknown-key suggestions** use an edit distance that counts adjacent transpositions as one
  edit, written in-tree rather than adding `strsim`. Unknown keys and enum values are recovered
  from serde's `unknown field` / `unknown variant` messages; a unit test guards that wording.
- **Document order is kept** for scopes, repos and `machine.toml` tables, so output is
  deterministic and follows the file.

## Stage 1: multiple stores and repo identity (#24)

- **`KIPI_STORE` is a `:`-separated list**, split like `PATH` (`std::env::split_paths`; empty
  segments skipped, each `~`-expanded). Flags, `KIPI_STORE` and `config.toml` `stores` each
  replace the sources below them as a whole; they never merge. The repeatable `--store` clap
  flag lands with the CLI foundation (#4); `config::store_paths` already takes the flag list.
- **`StoreSet` checks only what needs more than one store**: store, scope and entry names unique
  across stores, and references crossing into another store (`extends`, a repo's `scope`,
  `related`, bare `skills` / `exclude` naming another store's skill, qualified skills). Whether a
  reference within its own store names something that exists stays with resolution (#3), so an
  unknown name that exists in no mounted store is not a load error here.
- **Qualified skill references are checked completely at load**: the named store must be mounted,
  set `share_skills = true` and contain the skill. Consequently the example work store does not
  load alone (it uses `personal/commit-style`); the personal store does. A reference qualified with
  its own store's name (`acme/code-review` inside `acme`) is accepted and means the bare name.
- **Conflicts name both locations.** `Error::Conflict { at, other }` reports the later declaration
  (mount order, then file order) and prints the earlier one as `also at:`. Scope and repo references
  point at the referencing `[scopes.x]` / `[repos.x]` header, since the model keeps one location
  per table. Mounting the same directory twice is reported as such rather than as a name clash.
- **Every location in a store carries the store name** (`Location::store`, an `Arc<str>` shared by
  all of a store's locations), set by `load_store`. `StoreSet::provenance` renders it as
  `acme:bindings.toml:12` / `personal:skills/commit-style/SKILL.md`; errors keep printing full paths.
- **`RemoteUrl` normalization**: accepted schemes are `https`, `http`, `ssh`, `git`, `git+ssh`,
  `ssh+git`, plus scp-like `[user@]host:path`; `file://` and plain paths are local, not remotes
  (such a checkout matches by name only). User info and port are stripped, the host is lowercased,
  empty segments and trailing slashes dropped, and a trailing `.git` removed from the last segment.
  Path case is kept: self-hosted forges may treat it as significant. Nested groups keep every segment.
- **`remote` in `bindings.toml` must equal its own normalization** (checked by normalizing
  `https://<remote>`); a raw or non-canonical URL is rejected with the normalized value as the
  hint. `RepoEntry::remote` is now a `RemoteUrl`.
- **Checkout matching** (`model::match_checkout`): normalized `remote` → directory name (entries
  without `remote` only) → origin basename, where the basename step matches an entry whose *name*
  or whose `remote`'s basename equals it. The first rule with any candidate decides; more than one
  candidate is returned as `RepoMatch::Ambiguous`, never resolved by order. A basename match on an
  entry with a different `remote` is left for doctor to report.
- **`git::origin_url`** runs `git config --get remote.origin.url`; exit status 1 (unset) is `None`.
- **The personal example store no longer mentions the work store** in its comments, so loading it
  alone holds no work data (a test greps it for the work store's names and remote patterns).

## Stage 1: resolution engine (#3)

- **§15 open decision 1 (uncloned repos in the registry): list them.** A repo not cloned on this
  machine stays in its scope's registry with `local: None`, which the renderer prints as
  `(not cloned here)`.
- **§15 open decision 4 (default `grant`): `related`.** `DEFAULT_GRANT` stays `GrantMode::Related`;
  `grant = "registry"` is the documented opt-in for no prompts at all.
- **§15 open decision 5 (`@import` in conventions): no.** `always` files are flat; resolution
  hands their absolute paths to the renderer, which includes them verbatim, so every adapter
  renders the same text.
- **Placing a repo that is not cloned here.** §6 rule 2 (deepest root) needs a checkout path, so
  without one a repo with no explicit `scope` would always fall to `personal` / unscoped and drop
  out of its registry, which defeats decision 1. Between rules 2 and 3, a repo with **no checkout
  on this machine** is placed by its `remote`: the scope whose `remote_patterns` has the longest
  entry that is a prefix of the normalized remote. A tie between scopes decides nothing (fall
  through). A cloned repo is never placed by pattern, so doctor's "remote matches a pattern but
  resolved elsewhere" check keeps its meaning. Example: `feature-store` → `acme`.
- **Rule 3 (`personal`) and every other lookup stay within the repo's own store.** A work-store
  repo outside every root is unscoped even when the personal store (with its `personal` scope)
  is mounted.
- **Roots match whole path components** (`/w` contains `/w/x`, not `/wx`), compared lexically
  (no canonicalization: resolution does no I/O). Two scopes whose roots contain a checkout at the
  same depth is a hard error asking for an explicit `scope`. `machine.toml [roots]` replaces a
  scope's roots on this machine; it does not add to them.
- **Scope chain** of a repo = its scope's ancestors and descendants. Siblings (two scopes extending
  the same parent) are not in each other's chain. An unscoped repo's chain is "unscoped repos of
  the same store": two unscoped repos may be related; a scoped and an unscoped one may not.
- **The registry** of a repo is the nearest scope in its chain (itself first, then up) with
  `registry = true`; it lists every repo of that store whose scope is that scope or a descendant,
  including sibling scopes' repos. Registries live once on the `Plan` (sync writes one file per
  scope); a `RepoPlan` names its registry's scope (`Plan::registry_of`).
- **The `registry` grant is filtered by the scope chain.** §6 says the grant is "every cloned repo
  in the registry" and also that "every granted path must belong to a repo in the scope chain";
  where they differ (a sibling scope's repo) the leak rule wins and the sibling is listed in the
  registry but not granted. The `registry` grant also includes the `related` repos, so it is never
  narrower than `related`. A repo is never granted its own checkout.
- **`grant = "none"` grants nothing**, not even the store dirs (§6: "`none` → nothing").
  `related` and `registry` always grant the owning store's `repos/` and `skills/`, plus `skills/`
  of every store a (non-excluded) shared skill comes from.
- **`x-kipi.scope` is enforced.** A skill of the repo's own store with `x-kipi.scope: S` may only
  be bound by scope `S` or a scope extending it, or by a repo in one of those (an unscoped repo
  cannot use it); an `x-kipi.scope` naming no scope of the store is an error. Shared skills from
  another store are exempt: sharing is the explicit cross-store exception. `exclude` is never
  checked, since removing a skill cannot leak it.
- **Skill references are checked at every level** (scope `skills`, repo `skills` and `exclude`),
  including scopes no repo uses. Skills are identified by (store, name), so `acme/code-review`
  inside `acme` is the bare `code-review`; a duplicate keeps its first position and source.
- **`related` naming the repo itself is an error**; naming an unknown entry is an error with the
  closest name as the hint.
- **Precedence** for `agents`, `budget_tokens` and `grant`: repo, then the scopes from the repo's
  own up to the top-most, then the owning store's `[defaults]`, then the built-in default. The
  `Source` says which (`built-in default grant` has no location).
- **Provenance points at table headers.** The model keeps one `Location` per `[scopes.x]` /
  `[repos.x]` table, so a `Source` is that header's `store:bindings.toml:line` plus the key as the
  rule (`scopes.acme.skills`). Key-level lines can come later without changing `Source`.
- **Aliases**: keys are the entry name plus each alias, Unicode-lowercased, whitespace runs
  collapsed and trimmed, deduplicated per entry. A key claimed by two entries of one registry is
  an `AliasCollision` on the `Registry`; doctor (#12) turns it into an error. Aliases themselves are
  kept as written for rendering.
- **Deterministic output**: `Plan::repos` sorted by name, `registries` by scope, registry entries by
  name and a registry's scope list by name, grant repos by name after the store dirs. Declared
  order is kept where it carries meaning: skills (chain top-down, then the repo's, list order),
  `related`, `always`.
- **Test builders live in `kipi-test-support::builders`**, which now depends on `kipi-core`. Since
  that makes `kipi-core`'s unit tests see a second copy of its types, resolve tests are
  integration tests (`crates/kipi-core/tests/resolve.rs`, `resolve_props.rs`).
- **Coverage job** (`coverage (report-only)` in CI) runs `cargo +nightly llvm-cov --branch` over
  the resolve tests and writes the `resolve.rs` row to the job summary. Branch coverage is
  nightly-only, so the job uses an unpinned nightly and `continue-on-error`; it never gates.
