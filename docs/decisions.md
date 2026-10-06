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
- **`KIPI_STORE` is a single path for now.** Splitting it into a list, and repeatable `--store`
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
