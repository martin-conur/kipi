# Decisions

Choices the spec (`roadmap.md`) leaves open, recorded so later stages do not relitigate them.
The spec wins wherever it is explicit.

## Stage 0

- **YAML crate: `yaml-rust2`.** §12 allows `serde_yaml` or `yaml-rust2`; `serde_yaml` is
  deprecated upstream (`0.9.34+deprecated`), so frontmatter parsing will use `yaml-rust2`.
- **File lock crate: `fs4`** (superseded in #7: the apply engine uses `std::fs::File::try_lock`,
  stable since Rust 1.89, so no lock crate is needed).
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

## Stage 1: CLI foundation, `resolve` and `explain` (#4)

- **`--store <path>` is a top-level flag, given before the subcommand** (`kipi --store a resolve`).
  §7 also uses `--store` on `init` (a path), `add skill` and `tree` (a store *name*); a global
  `--store` would clash with those, so they keep their own flag. `--json`, `--no-color`, `-v` and
  `-q` are global and work anywhere on the line.
- **Usage errors exit 1, not clap's default 2**, which §7 reserves for doctor findings. `--help`
  and `--version` exit 0. `-v` and `-q` conflict.
- **Exit codes come from `kipi_core::error::ErrorKind`**: `Error::Io` is `Io` (exit 3), every
  other core error is `User` (exit 1). CLI-level failures (unknown repo, unimplemented command)
  exit 1; a failed write to stdout exits 3. Exit 2 is defined for `doctor` (Stage 3).
- **Unimplemented subcommands are declared with their §7 arguments** and fail with
  `` error: `kipi <cmd>` is not implemented yet `` plus the stage it lands in. Every writer
  (`init`, `stores`, `scan`, `add`, `bind`, `unbind`, `sync`, `doctor`) takes `--dry-run` through
  one shared `DryRun` arg group. `edit` has none: it only opens an editor. `completions` takes the
  shell as a string until Stage 4 adds `clap_complete`.
- **`-v` / `-q`**: `-v` reports the mounted stores on stderr; `-q` silences everything on stderr
  but errors. More detail per level comes with the commands that have something to say.
- **Colors**: on only when the stream is a terminal, `--no-color` is absent and `NO_COLOR` is unset
  or empty (no-color.org). stdout and stderr are decided separately. Explain's markers (`○ ● ✗`)
  and arrows (`←`, `→`) are plain Unicode text, not box drawing, so they are printed in pipes too;
  §7's ASCII fallback applies to `tree`'s box drawing (Stage 3).
- **Human output shows paths under the home directory as `~/…`**; JSON keeps them absolute.
- **JSON is built from view structs in `kipi-cli/src/output/json.rs`**, not by deriving
  `Serialize` on kipi-core's types, so the model can change without changing the documented shape
  (`docs/json.md`, `version: 1`). Sources print as `StoreSet::provenance` strings. `resolve <repo>
  --json` keeps the top-level shape with one repo and its registry; `explain <repo> --json` prints
  exactly that. Errors are never JSON.
- **Inherited vs explicit skills** are recorded by resolution as `PlannedSkill::origin`
  (`SkillOrigin::Scope(name)` / `Repo`) rather than inferred from the rule string, and exposed as
  `inherited_from` in JSON.
- **An unknown repo name** is a user error whose hint is the closest entry name, or, when the name
  is an alias, the entry it belongs to. Commands take entry names, not aliases (that is `where`).
- **Registry "size" in `explain` is its entry count** and the scopes it covers; token counts arrive
  with `tokens.rs` in Stage 3.
- **Provenance lines are table headers** (as decided in #3), e.g. `acme:bindings.toml:34` for every
  `repos.events-pipeline.*` key; built-in decisions print the rule only (`← built in`,
  `← grant: store \`acme\` dirs`).

## Stage 2: rendering (#5)

- **Rendering is pure; only the conventions are read.** `render::block::load_conventions` reads
  a repo's `always` files up front (an I/O error names the `scopes.<s>.always` key to fix);
  `render::registry::render`, `render::block::render` and `render::splice::splice` are text in,
  text out. The home directory is a parameter, so `~/…` display never reads the environment.
  `display_path` moved from the CLI into `kipi_core::render` and the CLI re-exports it.
- **Templates are minijinja, embedded from `templates/`** (`block.md.j2`, `registry.md.j2`, and
  `macros.md.j2` holding the one `repo_line` macro both share, so a registry line is identical in
  the file and in the block). Strict undefined, no auto-escaping, `trim_blocks` + `lstrip_blocks`.
  Templates are compiled into the binary, so a template failure is a kipi bug and panics, with
  every branch covered by the snapshot tests.
- **Registry line**: `- name (alias, …) — summary · path · notes: path`. Aliases are omitted
  when empty, the summary when absent, the notes when there are none; a repo not cloned here shows
  `(not cloned here)` in place of the path. A repo without a summary still lists its aliases:
  they are lookup keys, and §6's "name only" is read as "no summary text". Draft markers are not
  rendered; they are for `tree` and `doctor`, not the agent.
- **Registry file** `generated/registry.<scope>.md` = a one-line generated-by comment naming the
  store, `# Known repos (<scope>)`, a blank line, then the lines. The block's `## Known repos`
  section is the same lines.
- **Conventions**: one `## Conventions (<scope>)` heading per scope, top-most first; several
  `always` files of one scope join under one heading with a blank line between them; trailing
  whitespace is trimmed and a file that is empty after trimming adds nothing. A conventions file
  containing a kipi marker line is an error (it would cut the block short).
- **This repo section**: `Scope: <s>.` (`none (unscoped)` for an unscoped repo, which gets no
  conventions and no registry), then `Related: …` with each repo's access: `read access granted`
  (its path is in the grant), `not cloned here`, or `read access not granted` (e.g. `grant =
  "none"`). When every related repo has the same status it is printed once after the list, as
  in §8. A related repo the known-repos list does not show (no registry, or it lives in an ancestor
  scope above the registry) gets its own line with path and notes, so the agent still finds it.
  The "read its code at the listed path" hint appears only when some repo is listed.
- **Markers**: the begin marker is any line starting with `<!-- kipi:begin` (the text after it is
  free), the end marker a line that is `<!-- kipi:end -->` plus optional trailing whitespace. A
  marker anywhere but the start of a line is ordinary text. The replaced region runs from the
  begin marker's first byte to the end marker's last, so line endings (CRLF included) and anything
  around the block stay byte-identical.
- **Splicing**: no file or an empty file → the block plus a newline. No markers → the block
  appended after exactly one blank line (nothing added when the text already ends in a blank line;
  existing bytes are never rewritten). One begin before one end → replaced in place. Two begins,
  two ends, a lone marker or an end before its begin → an error at the offending line asking for
  exactly one pair.
- **Token estimates** are checked with `tiktoken-rs` (`cl100k_base`, a dev-dependency until
  `tokens.rs` lands in Stage 3) against §3 / §7: acme conventions ≤ 300, acme registry ≤ 350,
  personal registry ≤ 210, events-pipeline block ≤ 700 (≈ 1.3k minus ~600 for skill descriptions).
- **Snapshots use `examples/` as the home directory**, so store paths print as `~/acme-store/…`
  on every machine while the example `machine.toml` checkouts (`/Volumes/…`, `/srv/…`) stay
  absolute.

## Stage 2: `kipi scan` (#22)

- **What is walked.** The scope roots of every mounted store (a `machine.toml [roots]` line
  replaces a scope's own roots), or only the paths given on the command line. Each root is walked
  on its own, so a root nested inside another root (or inside a checkout) is still reached; found
  checkouts are deduplicated by path. A checkout is a directory holding a `.git` *directory*; a
  `.git` file (a worktree or submodule) is neither reported nor descended into. Found checkouts are
  not descended into, and hidden directories, `node_modules`, `target` and `.venv` are skipped.
  Symlinks are not followed; paths are kept as walked (not canonicalized), like the roots.
- **Identity.** A path already recorded in `machine.toml [repos]` belongs to that entry. Otherwise
  the shared `match_checkout` decides (remote, then directory name for entries without `remote`,
  then origin basename), with one restriction: **a basename match only counts for entries without
  a `remote`.** An entry with a different `remote` is a different repository that shares the name,
  which is exactly the case the issue names as `<owner>-<name>` (the personal `api` vs a work
  `api`). Matching it would record a work checkout against a personal entry. So "a checkout whose
  remote differs from its entry's" is reported only for a checkout recorded by path. Several
  candidates under one rule are reported (`ambiguous-match`) and nothing is written for them.
- **A recorded checkout of an unmounted store's entry** is left alone, never added to whichever
  mounted store's root happens to contain it.
- **Machine.toml writes.** A new line per identified checkout. A recorded path that no longer
  exists is replaced by the checkout found now (reported as moved); otherwise a vanished path is
  reported and kept (scan removes nothing). Several checkouts of one entry are reported; the
  recorded one is kept if it still exists, else nothing is written.
- **New entries.** The scope is the deepest root containing the checkout across every mounted
  store; the entry goes to that scope's store, with no `scope =`. A checkout outside every root, or
  under two scopes' roots at the same depth, is reported (with a `kipi scan <path> --store
  <s> --scope <scope>` hint built from `remote_patterns` when they match) unless `--scope` is given:
  then it goes to that scope's store with `scope = "<scope>"` written. `--store` only checks that
  it owns `--scope` (scope names are unique across stores). Two clones of one new repo are
  reported and not added.
- **Entry names**: the directory name (else the remote's basename); if another entry has it,
  `<owner>-<name>` with the remote's owner (second-to-last path segment); otherwise reported. New
  checkouts are processed in path order, so the same layout names the same entries on every
  machine. Aliases of a `<owner>-<name>` entry are drafted from `<name>`.
- **`remote_patterns` conflicts** are warned for every identified or added checkout: its remote
  matches a scope's pattern (longest wins; a tie decides nothing) but its entry resolves to a scope
  that is neither that scope nor one extending it (so an `acme-ml` repo matching `acme`'s pattern
  is fine), or lives in another store. The hint is the `scope =` line, or the store to move to.
- **A work remote never lands in another store** (#46, §1 principle 5). A new checkout whose
  remote's longest `remote_patterns` match (a tie counts if any tied scope is in another store)
  belongs to a store other than the one its root, or `--scope`, places it in is **reported, not
  added**: `foreign-remote`, naming that store and the fix (add the directory to one of its
  roots, or `kipi scan <path> --store <s> --scope <scope>`, or narrow the pattern). Nothing is
  recorded about it, not even in `machine.toml` (there is no entry to record). We considered
  adding it straight to the pattern's store with `scope = "<s>"`, and chose not to: a pattern can
  be too broad, and filing a repo into the work store is the user's call. To make the hint
  work, `--scope` also places a checkout whose root's store is claimed this way, not only one
  outside every root. An existing entry identified in the wrong store keeps its
  `scope-conflict` warning and no longer gets the work `remote` filled in or drafts refreshed.
  With only the personal store mounted there is no pattern to check, so the docs recommend
  narrow personal roots on machines that hold work checkouts.
- **Drafting** (§9): manifest description first (`Cargo.toml` `package.description`, then
  `pyproject.toml` `project.description` / `tool.poetry.description`, `package.json`, the `//`
  comment block right above `go.mod`'s `module`); a non-string value such as
  `description.workspace = true` is skipped. Else the first README paragraph (`README.md`,
  `README.markdown`, `README`, `README.txt`, case-insensitive) that is prose after removing
  headings (ATX and setext), code fences, HTML comments and HTML headings, images (badges), lists,
  tables and reference definitions, keeping link and code text and dropping bare URLs; it needs two
  words. Whitespace is collapsed, a single trailing period dropped, and text over 140 chars is cut
  at the last word boundary before it with `…`. Key-file hints are appended with `; `: `OpenAPI in
  <path>` (`openapi.{yaml,yml,json}` at the root or in `api/`) and `dbt project`
  (`dbt_project.yml`), unless the summary already mentions OpenAPI / dbt. No language hint: a
  manifest already says what language the repo is, and the hint would fight the example summaries.
- **Aliases**: the name split on `-`, `_` and `.`, lowercased; generic words (`api`, `service`,
  `lib`, `cli`, `app`, `web`, `tool`, ... see `GENERIC_WORDS`), one-letter, numeric and `v2`-style
  parts dropped, and any key (name or alias, case-folded) another entry already has. A one-part name
  gets no aliases (the name already resolves). An empty result writes no `aliases` key, since an
  empty list is invalid.
- **Draft markers** are written as `  # kipi:draft` trailing comments. `--refresh-drafts` replaces
  a still-marked value and keeps its decor (so the marker stays); it never removes a value, and an
  empty re-draft leaves the old one. `remote` is factual and gets no marker. A filled `remote` is
  appended to the end of the entry's table rather than moved to the top, so comments attached to
  the first key stay where they are.
- **Example store**: `feature-store`'s drafted aliases are now `["feature", "store"]`, what scan
  drafts, so every `# kipi:draft` value in the examples is reproducible (tested by scanning the
  example checkouts with `--refresh-drafts` and expecting no change).
- **Writes** are computed in full first, then written atomically per file (temp file + rename);
  a file is only written when its text changes, which is what makes a second run a no-op.
  `--dry-run` prints a unified diff per file (`output/diff.rs`, LCS over lines, 2 lines of
  context); no diff crate is added.
- **Output**: the report goes to stdout (warnings included, as `warning:` / `hint:` pairs, since
  they are the command's result); `--json` prints the report documented in `docs/json.md`
  ("Scan"). Warnings do not change the exit code (0); doctor owns exit 2.

## Stage 2: adapters (#6)

- **Trait.** `AgentAdapter` follows §12 with three adjustments. `id()` returns `AgentId` (its
  `as_str()` is the `"claude"` / `"generic"` of §12), so callers match on an enum.
  `render_skill_entry` takes the `PlannedSkill` the `RepoPlan` already holds plus the repo
  directory (the §12 `target`), and returns an absolute `Link`. Methods are added:
  `local_always_on_file` (decision 3 below), `exclude_patterns`, so the `.git/info/exclude`
  entries for what an adapter writes live in its constant block with its paths, and
  `reads_when_absent` / `import_line` (the AGENTS.md rule below).
- **Actions live in `plan.rs`.** `Action` is `Link`, `WriteBlock { file, text }` (the
  adapter-rendered block, placed by `render::splice::splice_text`), `MergeGrant { file, key,
  entries }`, `Remove` and `Exclude`. Adapters never emit `Remove`; the apply diff (#7) does.
  `plan::repo_actions(repo, block, checkout)` returns, per enabled adapter in the repo's `agents`
  order, the block write, then one link per effective skill, then the grant; plus one `Exclude`
  with every adapter's patterns (deduplicated, in order) and `.kipi/`. A repo not cloned here
  yields `None`.
- **The Claude grant is always emitted**, even with no directories, so a repo whose grant shrinks
  to nothing still tells the apply engine which entries to drop; apply should not create the file
  for an empty list. The generic adapter has no grant mechanism (`None`).
- **§15 decision 2, AGENTS.md by default: written whenever the generic adapter is enabled**, which
  it is by default (`defaults.agents = ["claude", "generic"]`). The spec's "or the file already
  exists" is not adopted: `agents` without `generic` is an explicit opt-out, and writing AGENTS.md
  anyway would override it. Pure planning also stays free of filesystem checks.
- **§15 decision 3, tracked always-on files: never dirty a tracked file without
  `--allow-tracked`.** Claude Code loads `CLAUDE.local.md` beside `CLAUDE.md`, so a tracked
  `CLAUDE.md` sends the block to `CLAUDE.local.md` (added to `.git/info/exclude`). AGENTS.md agents
  have no additive untracked file (`AGENTS.override.md` replaces `AGENTS.md`), so a tracked
  `AGENTS.md` gets no block and a notice with a hint; so does a tracked `CLAUDE.local.md`.
  `--allow-tracked` writes into the tracked file. Which files are tracked is passed in
  (`plan::Checkout`), so `repo_actions` stays pure; sync (#8) fills it from `git ls-files`.
- **Never make Claude Code drop the user's AGENTS.md.** By default Claude Code reads `AGENTS.md`
  only while neither `CLAUDE.md` nor `CLAUDE.local.md` exists. So when the Claude adapter would
  *create* its file (no user content in either CLAUDE file) and the repo has a user `AGENTS.md`:
  (a) if another enabled adapter writes the block into that `AGENTS.md` (generic on, file not
  skipped as tracked), Claude writes no always-on file and reports a notice, since Claude keeps
  reading `AGENTS.md`, now with the block; (b) otherwise (generic off, or `AGENTS.md` tracked) the
  new `CLAUDE.md` carries `@AGENTS.md` as the first line inside the block, so Claude still loads the
  user's file, and the block exists only once because nothing writes it into `AGENTS.md`. The import
  sits inside the markers, so kipi owns it and removes it with the block. (a) is preferred because it
  adds no file. A user file is one present on disk that kipi did not create, or that it created and
  the user has since edited outside the block (`plan::Checkout::user_files`, filled by sync from the
  disk and the manifest), so a kipi-created `AGENTS.md` / `CLAUDE.md` pair stays stable across
  syncs. The rule is generic: adapters declare `reads_when_absent` and `import_line`. Parent
  directories are not inspected.
- **Golden tests** (`tests/adapters.rs`): one snapshot per adapter per example repo with the
  block target, links (store paths as `~/…`), grant entries, exclude patterns, notices and the
  block text. A disabled adapter or a repo not cloned here is a one-line snapshot.
- **Size rule**: a test counts lines of every `adapters/*.rs` except `mod.rs` (shared trait and
  helpers) and fails at 100.

## Stage 2: apply engine (#7)

- **One engine, many targets.** `apply::run(target, actions, options)` takes a `Target` (a root
  every action path must live under, plus its manifest file) and returns a `Report` of changes,
  skipped actions (each with a hint) and foreign entries. A consumer repo is
  `Target::repo(dir)` with `.kipi/manifest.toml`; a store's registries are
  `Target::generated(store)`, rooted at `generated/` with `generated/.kipi-manifest.toml`, so a
  registry whose scope drops `registry = true` is removed exactly like a stale link. Registries are
  `Action::WriteFile` (new variant: a whole file kipi owns), built by `apply::registry_action`.
  Sync (#8) must call `run` with an empty action list for a repo (or store) that no longer has a
  plan, so unmounting a store or unbinding a repo cleans up through the manifest.
- **Manifest** (`manifest.rs`, `version = 1`): paths relative to the target root. `[[link]]`
  path + absolute target; `[[block]]` path, `created` (kipi created the file) and the block's
  `sha256:` hash; `[[file]]` path + hash; `[exclude]` patterns; `dirs` kipi created; `[[grant]]`
  file, key and owned entries, defined here and carried over untouched until the grants engine
  (#23) fills it. Unknown keys or another version are errors whose hint is "delete the manifest
  and re-run" (everything it listed is then foreign and left alone). An empty manifest is deleted,
  with `.kipi/`.
- **Three-way diff, per kind.**
  - Link: missing → created; a symlink to the same target → kept (adopted into the manifest if it
    was not there, so a lost manifest heals); a symlink elsewhere → replaced atomically (temp link
    + rename); a regular file or directory → skipped unless `--force`, which removes it first.
    Removal deletes the link only if it is still a symlink to the recorded target; anything else is
    reported and left in place. Deleting uses `remove_file` on the link, which never touches the
    target.
  - Block: spliced as in rendering; written only when the text changes. Removal takes the block out
    (`render::splice::remove_block`, the inverse of appending: text that ended in one newline gets
    back byte for byte) and deletes the file only when kipi created it and nothing but whitespace is
    left. A symlinked or directory always-on file is skipped with a hint: kipi never writes through
    a symlink (a `CLAUDE.md -> AGENTS.md` setup keeps working, with the block in `AGENTS.md`).
  - File: written when missing, kipi's, or with `--force`; removed only when its hash still matches.
  - Exclude: one marker section in `.git/info/exclude` (``# kipi:begin (managed by `kipi sync`; do not
    edit)`` … `# kipi:end`), replaced in place or appended; removed with the last action. Located with
    `git rev-parse --git-path info/exclude` when `.git` is a file (worktrees, submodules).
  - **The always-on files kipi creates are excluded too** (#48, §4 gitignore policy). §4 lists
    only the adapters' directories and `.kipi/`, which left a kipi-created `CLAUDE.md` /
    `AGENTS.md` (just the block, with its absolute local paths) showing as `??` and one
    `git add .` away from a commit. So an adapter's own always-on file joins the section when it
    is absent (the block write creates it) or the manifest records it `created`, and git does not
    track it. The pattern is anchored (`/CLAUDE.md`, `/AGENTS.md`): a `CLAUDE.md` in a
    subdirectory is the user's. A file the user had before (`created = false`) is never excluded;
    one kipi created stays excluded after the user adds text outside the block, since the block
    is still machine-specific. Once the user tracks a kipi-created file (`git add -f`), the
    pattern is **dropped** (it would be harmless, but kipi no longer owns the file's place in
    git), K022 reports the tracked block, the next sync moves it as for any tracked file
    (`CLAUDE.local.md`; nothing for `AGENTS.md`), and removal leaves the file in place instead of
    deleting a tracked file kipi happened to create. The lines go with the files: unbinding or
    unmounting removes both. With the #6 AGENTS.md rule, a user `AGENTS.md` that carries the
    block is the user's and stays unexcluded, and a kipi-created `CLAUDE.md` importing it is
    excluded. K021 needs no change: it checks what the manifest recorded.
  - Grants: merged into the settings file's JSON array; see "Stage 2: read grants (#23)".
- **Directories.** Missing parents are created one by one and recorded; a recorded directory that no
  wanted path lives under is removed once empty (deepest first) and stays recorded while it still
  holds something kipi did not write. Nothing is written or removed below a directory that is a
  symlink (or a file): the action is skipped, and an old entry there is dropped, since acting on it
  would act on whatever the symlink points at.
- **Foreign** entries are what sits in a directory holding kipi's links without being in the
  manifest or wanted (a hand-installed skill); reported, never touched.
- **Crash safety.** Before touching the disk, `run` writes the union of the old and new manifests,
  so an interrupted run leaves nothing unrecorded; the final manifest follows. Each file write is a
  temp file plus rename (`apply::write_atomic`, now shared with `scan`).
- **Dry run** plans the same steps and returns their changes without executing any. The only
  possible difference from a real run is a directory predicted empty that turns out not to be.
- **Skips are not errors.** A refusal (file in the way, malformed markers, a symlinked file) is a
  `Skipped` with a hint and the rest of the actions still apply; errors are I/O failures and
  actions outside the target root (a kipi bug).
- **`Checkout`** is filled by `apply::checkout(dir, allow_tracked)`: the always-on files of every
  adapter (`CLAUDE.md`, `CLAUDE.local.md`, `AGENTS.md`) that git tracks (`git ls-files`), and those
  holding user content: present and not created by kipi per the manifest, or created by kipi but
  with non-whitespace text outside the block (or a symlink, or unreadable).
- **Lock: `std::fs::File::try_lock`** on `<config dir>/lock` (stable since Rust 1.89, an advisory
  `flock` on Unix) instead of `fs4`, which is then not needed. It fails at once with a hint when
  another process holds it, and is released on drop or exit. `apply::Lock::acquire` is for sync
  (#8) to hold across all targets.

## Stage 2: read grants (#23)

- **Owned-entry merge** (`json_merge::merge`): kipi adds the entries the plan grants and removes
  only the entries the manifest's `[[grant]]` records as its own. An entry already in the array
  that kipi did not add is the user's, even when kipi wants the same path: kipi does not claim it,
  so it never removes it later. Non-string entries and every other key are left alone.
- **Order.** New entries are appended; entries already there keep their place, so a mode switch
  shows as the lines it adds or removes rather than a reshuffle. Key order is kept by enabling
  `serde_json`'s `preserve_order` feature (in the §12 set already; it pulls in `indexmap`, which
  `toml_edit` already depends on). The file is re-indented with the indentation it already uses
  (two spaces for a new file) and keeps or omits its final newline. Comments are not JSON and
  fail to parse, so a file with them is reported as malformed.
- **Idempotence** is decided on the parsed document, not the text: when the document would not
  change, nothing is written, whatever the file's formatting.
- **Clean removal.** When removal empties the array, the array is dropped, and so is every object
  on the way to it that this leaves empty. `[[grant]]` gains `created` (kipi created the file;
  `serde(default)`, so `version = 1` manifests still load); a created file left as `{}` is
  deleted, so `grant = "none"` or unbinding returns the checkout to how it was. A file the user
  had is kept, even as `{}`.
- **Refusals** (reported in `Report::skipped` with a hint, file untouched, old `[[grant]]` kept so
  a later run can still remove kipi's entries): malformed JSON or a value of the wrong type on the
  key path; a settings file that is a symlink or a directory; a settings file git tracks (Claude
  Code then holds the grant until the folder is trusted, see `docs/adapters.md`, and writing it
  would show in `git diff`); a symlinked `.claude` (the usual rule: nothing is written through a
  symlinked directory).
- **No empty scaffolding.** With nothing to grant (`grant = "none"`), kipi creates neither the file
  nor `.claude/`.
- **Dry run.** `Report::grants` lists each settings file whose grant changes, with the entries added
  and removed and the whole text before and after, for `sync --dry-run` (#8) to print as a diff.
  The change itself is `What::Grant`: create (new file), update (entries added or removed, file
  kept), remove (file deleted), the same convention as `What::File`.
- **Manifest.** The final manifest records a `[[grant]]` only while kipi owns entries in that
  file; a cleared grant leaves none behind, so a path the user later adds by hand (or a settings
  file they write after kipi deleted its own) is theirs. The crash-safety manifest written before
  the disk changes combines old and new entries per file, since entries being removed are still on
  disk until the write.
- **Limit of ownership by value.** While the manifest records a path as kipi's, a copy the user
  adds by hand before the next sync cannot be told apart from kipi's own entry, and goes when
  kipi drops the grant. Once a sync has cleared it, the path is the user's for good.
- **Codex CLI** gets no grant: see `docs/adapters.md`.

## Stage 2: `sync` and `status` (#8)

- **One orchestrator, two commands.** `kipi_core::sync::run(stores, machine, plan, selection,
  options)` renders each selected repo's block, reads its `Checkout` (`apply::checkout`), builds the
  adapters' actions (`plan::repo_actions`) and applies them; it also applies every mounted store's
  registries (`Target::generated`). `status` is the same call with `dry_run`, so "synced" means
  exactly "a sync would change nothing" and the two can never disagree.
- **Registries are regenerated on every sync**, named repos or `--all` (roadmap §8 step 2 is
  unconditional, and they are cheap). A store whose scopes no longer enable a registry gets an empty
  action list, so its old file goes through the generated manifest.
- **Orphans (`--all` only).** A `machine.toml [repos]` checkout whose entry is in no mounted store
  (the entry was deleted, or its store unmounted) and that still has a `.kipi/manifest.toml` is
  applied with no actions, so everything kipi wrote there is removed (roadmap §6, "Unmounting a store
  and re-syncing removes everything it contributed"). A checkout that a live entry also points at (a
  renamed entry whose old name lingers in `machine.toml`) is never treated as an orphan. Named
  syncs never touch orphans: a name must be a live entry.
- **Skipped, not failed.** An entry with no checkout in `machine.toml` is reported as not cloned; a
  recorded checkout that is not a directory is reported with a hint to re-run `kipi scan`. Neither
  is an error, and nothing is created at a missing path.
- **`--agents a,b` replaces each repo's `agents` for the run**, like any other plan input: the diff
  then removes what the other adapters wrote, and the next plain sync puts it back. Unknown names
  are user errors with a did-you-mean hint. `--allow-tracked` (§15 decision 3) is a sync flag.
- **`sync` requires repo names or `--all`** (they conflict); a bare `kipi sync` is a user error with
  a hint rather than an implicit `--all`, since it writes into every checkout. The lock
  (`apply::Lock`) is held for the whole run unless `--dry-run`; `status` takes no lock.
- **Always-on size** is estimated over the rendered block (without an adapter's import line) plus
  the `description` of each effective skill (§10), as characters ÷ 4 (`tokens::estimate`) until the
  tokenizer lands in Stage 3. Over budget is a warning in the sync summary and `over` in status.
  It is the cost to one agent session, so it is computed from the plan before any `--agents`
  override and does not change with the enabled adapters (they all write the same block and skills).
- **Status states.** `synced` (dry run changes nothing), `stale` (a manifest exists and the dry run
  changes something), `missing` (cloned here, no manifest: kipi's files are missing), `not_cloned`,
  `checkout_missing`. "Missing" in §7 is read as "never synced": a repo that is not on this machine
  has nothing to be missing from, and gets its own state. `status` also counts the repos whose
  summary is still `# kipi:draft`.
- **Summary counts** (`sync::Tally`): links created or replaced / removed; blocks written / removed
  (a kipi-created block file deleted with its block counts as one block removed); read-grant
  entries added / removed (from `Report::grants`); whether the exclude section changed. Dry runs
  also list every change and print the settings-file JSON diff.
- **Onboarding E2E** mounts the stores with `kipi init --store <existing store>` (since #9).

## Stage 2: authoring commands (#9)

- **Validate, then write.** Every authoring command computes its edits first
  (`kipi_core::authoring::Outcome`, nothing written). An edited `bindings.toml` (and, for `add
  repo`, `machine.toml`) is re-loaded from its new text (`config::load_store_text`) into the mounted
  set and resolved; any error rejects the whole edit and is reported with `(rejected: nothing was
  written)` appended. That is how a bind crossing scopes or stores (the leak rule), a missing skill or
  a store-name clash on `init` is refused. Argument errors that are about no file (unknown repo,
  invalid skill name, no `--skill/--related/...` given) are a new location-less `Error::Usage`
  (exit 1, with a hint). Writes take the apply lock; `--dry-run` prints a unified diff per file
  (shared with `scan`), plus the directories it would create and the files it would copy.
- **Comment-preserving array edits** (`authoring/toml.rs`). `bind` appends to the entry's list,
  creating `key = [...]` at the end of the entry when absent; a new element copies the decor of the
  last one, so multi-line arrays stay one value per line. `unbind` removes the value; a list left
  empty is removed with its line. So bind followed by the same unbind is byte-identical whenever the
  key was absent or non-empty (tested for every example repo); the one exception is an explicit
  empty `skills = []`, which bind+unbind removes (an empty list means the same as none).
- **Values already there / not there** are reported as `keep` and change nothing (a repeated bind
  is a no-op). Aliases match case-folded, as the registry matches them; everything else exactly.
  Unbinding a skill the repo still inherits from its scope reports the `kipi bind <repo> --exclude
  <skill>` to use instead.
- **Aliases**: editing `aliases` through `bind`/`unbind` clears its `# kipi:draft` marker (roadmap
  §5.1: a human edited it), so that round trip is not byte-identical by design. An alias that
  would collide with another entry's name or alias in the same registry is rejected (a `doctor`
  error otherwise).
- **`init`**: default path `~/.agents/store`. Default name: `personal` for the default store,
  otherwise the directory name without a leading `.` and a `kipi-` prefix (`~/work/.kipi-acme` →
  `acme`); `--name` overrides. A new store gets `skills/`, `repos/`, `conventions/`, `generated/`,
  the `bindings.toml` skeleton (its commented example scope included) and a `.gitignore` holding
  `generated/` (appended to an existing one, never replaced). Pointed at an existing store it reads
  it, refuses a different `--name`, and only mounts it: no store file is touched. `--git` runs
  `git init` unless the store already has a `.git`. Mounting appends the path to `config.toml`
  `stores` written `~/…` under the home; when `config.toml` lists no stores yet and the default
  store exists, the default is listed first, since listing only the new store would silently
  unmount the implicit default. Already listed (same path, or the same directory once
  canonicalized) is a no-op, which makes `init` idempotent.
- **`stores`** lists name, path, git state (`git status --porcelain -- .` in the store, and whether
  the repository has a remote; `local, no git` outside a work tree), scopes, repo count and
  `shares skills`. `--unmount` takes a store name or path, removes that `config.toml` entry and
  keeps the rest of the file (an emptied list stays as `stores = []`, which mounts the default
  store, and says so). Only stores listed in `config.toml` can be unmounted; the next `sync --all`
  cleans up the unmounted store's repos through their manifests (orphans, #8).
- **`add skill`**: the store is `--store`, else the store owning `--scope`, else the only mounted
  store, else a usage error. The skill name must be a valid Agent Skills name and the directory must
  not exist (kipi never overwrites a skill). From the template, `x-kipi.scope` is filled when
  `--scope` is a scope of the skill's own store and left out otherwise. `--from <dir>` must hold
  `SKILL.md`; a git URL (`scheme://…` or `user@host:…`) is shallow-cloned with the `git` binary into
  a temp dir and the skill taken from its root, `skills/<name>/` or `<name>/`. The imported
  frontmatter is validated and its `name` must equal `<name>`. The tree is copied (`.git` skipped,
  symlinks kept as symlinks) into a hidden staging dir and renamed into place. With `--scope`, the
  skill is appended to that scope's `skills`, as `<store>/<name>` when the scope belongs to another
  store (resolution then requires `share_skills`). `--link-back` (needs `--from <dir>`) replaces the
  imported directory with a symlink to the store copy after the copy succeeded.
- **`add repo`**: the entry name must be free in every mounted store. `--path` must be a directory
  not already recorded in `machine.toml`; its origin, normalized, becomes `remote` and must not
  already belong to another entry (that is the same repository; `scan` records it). The store is
  the one owning `--scope`, else the scope whose root holds `--path`, else the only mounted store.
  `scope =` is written only when `--scope` disagrees with (or there is no) path placement, as in
  `scan`. `--summary` is written as reviewed (no draft marker). `--path` is recorded in
  `machine.toml`.
- **JSON**: `stores --json` and every authoring command's `--json` report are documented in
  `docs/json.md` ("Stores", "Authoring").
- **`tempfile`** moves from kipi-core's dev-dependencies to its dependencies (it is in the §12
  list) for the clone directory of `add skill --from <git-url>`.

## Stage 3: doctor framework and config/resolution checks (#12)

- **Doctor reports what would stop loading, instead of stopping.** A reference into another
  store, a missing skill or a `related` repo outside the scope chain makes `StoreSet::new` or
  `resolve` fail on the first one. Doctor loads each store on its own, collects every bad
  reference (`StoreSet::checked` returns them all; `new` still returns the first, so every
  other command's errors are unchanged), reports each as a finding and removes it
  (`RefSite::remove_from`), then resolves what is left and runs the plan-based checks. The
  order is fixed: cross-store references and qualified skills (K012 / K005), then skills
  missing from their own store (K005), then `related` across scopes (K003, judged on a plan
  resolved with every `related` list emptied, since relations never decide a scope).
- **What has no check stays an error.** Store/scope/entry name collisions, malformed files, a
  cycle or unknown parent in `extends`, an unknown explicit `scope`, `x-kipi.scope` misuse, a
  `related` name that is unknown or the repo itself: doctor prints the usual `error:` /
  `hint:` and exits 1, as every other command does.
- **`Finding::fix` is a doctor `Fix`, not a plan `Action`.** §12 sketches `fix: Option<Action>`,
  but the only safe fix in this ticket (K015, fill `remote`) edits `bindings.toml`, which no
  `Action` expresses. `Fix::FillRemote { store, repo, remote }` is the first variant; #13 adds
  the filesystem ones (and may wrap `Action`). `--fix` is a user error ("not implemented yet")
  until then.
- **Stable codes K001–K015**, one per §10 row this ticket covers, in the issue's order (Rev 2.1
  rows last). A code's level is fixed by the code. Documented in `docs/doctor.md`; a test keeps
  the doc and `Code::ALL` in step. #13's checks continue from K016.
- **Two §10 rows share K014** ("two checkouts matching one entry, or a checkout whose remote
  differs"), as the table does. "Matching" uses `scan`'s rules (`match_checkout`) over every
  `machine.toml [repos]` checkout that exists; only a unique match counts.
- **Remotes.** K001 and K013 identify a repo by its checkout's origin when it is cloned here
  (that is what §10 says: "the repo's `remote.origin.url`"), else by the entry's `remote`.
  Origins are read with `git config --get remote.origin.url`; a missing checkout is skipped
  (it is #13's "path no longer exists"), and a directory git cannot read or a local-path origin
  counts as no origin. K001 accepts a matching pattern on any scope of the repo's chain (a
  repo in `acme-ml` with an `acme` remote is fine) and suggests the longest matching pattern's
  scope. K013 fires only when no scope of the repo's own store matches.
- **K002 is the `personal` fallback catching a stray checkout.** A cloned repo can only land in
  a scope without a containing root through rule 3 (a cloned repo is never placed by pattern),
  so the check is: cloned, no explicit `scope`, and no root of its scope (after
  `machine.toml [roots]`) holds the checkout. An explicit `scope` silences it.
- **K003 also checks the plan's grants**, although resolution never grants across scopes or
  stores: a repo dir outside the chain, another store's `repos/`, or the `skills/` of a store
  none of whose skills the repo uses. The hint calls such a finding a bug.
- **K006** matches `/Users/<name>` and `/home/<name>` (as the leak guard does, without its
  placeholder allowlist: any home path in a store is wrong) and this machine's home directory
  as a whole path component, in `bindings.toml`, `conventions/*.md`, every scope's `always`
  file and every `repos/<name>/notes.md`. One finding per file, at its first such line.
- **`kipi doctor <repo>`** keeps only findings whose `repo` is that entry; an alias collision
  is reported against the named repo when it is one of the claimants. Store-level findings (a
  scope's missing skill, home paths outside notes, skill descriptions, registry size) are left
  out. An unknown name is a user error with the usual did-you-mean hint.
- **Output.** Findings are sorted by level, code, then repo (store-level first). Text prints
  `level[code] repo: message`, `--> store:file:line` and `hint:`, then a count line; `--json`
  prints `{ version, findings, summary }`. With an error-level finding the report is still
  printed in full on stdout, and stderr gets one `error:` / `hint:` line, exit 2.
- **Notes size** uses the character estimate (`tokens::estimate`) until the tokenizer lands
  (#10); 1,500 tokens is 6,000 characters for now.
- **The example `machine.toml` overrides `personal`'s roots** (`/srv/code`), like it already
  did for `acme` and `acme-ml`. Without it doctor rightly warned (K002) that the four personal
  checkouts sit outside `~/code`. The resolve test now exercises the fallback by dropping that
  override.

## Stage 3: agent-facing lookup — `where`, `grep`, `edit` (#11)

- **What a repo may see is its scope chain: its scope, the scope's ancestors and its
  descendants** (`search::View`), the same rule `related` obeys (§6), within the repo's own store.
  An unscoped repo sees the unscoped repos of its store (resolution counts those as one chain).
  Shared skills the chain binds are searched like any other bound skill; nothing else of the
  sharing store is. Sibling scopes are out of view even when a common ancestor's registry lists
  them, so `where`/`grep` never widen what `related` may reach.
- **The view comes from the working directory.** The repo whose checkout holds the cwd (deepest
  match, symlinks resolved) sets it; outside every checkout both commands see every mounted store
  (a human at a shell). `where --any-scope` and `grep --scope <s>` override. A not-found `where`
  suggests near names only from inside the view, so a miss never names a repo outside it.
- **`where` matching**: an exact entry name wins; otherwise aliases (case-folded, whitespace
  normalized, §6). Several alias matches are an error listing the entries (exit 1); `doctor` reports
  the collision (K004). Output is one fact per line with absolute paths and no colors; `--notes`
  appends the notes, and is a user error when the repo has none (hint: `kipi edit notes:<repo>`).
- **`grep`** searches, in order, the chain's `always` files, the notes of its repos, its skill
  directories, and with `--repos` the checkouts cloned here. Walks use `ignore` like ripgrep:
  `.gitignore`, `.git/info/exclude` (so kipi's own skill links in a checkout are not searched
  twice) and hidden files are skipped, symlinks are not followed, files over 1 MiB and files with
  a NUL byte in their first 8 KiB are skipped, unreadable files are skipped silently. Patterns are
  Rust `regex`, case-sensitive (`(?i)` to ignore case). `--limit` (default 100) caps the lines;
  when it cuts, a `note:`/`hint:` goes to stderr so stdout stays pure `path:line: text`. No match
  exits 0 with a note on stderr (exit 1 is reserved for user errors). Long lines are cut to 300
  characters.
- **`edit` targets**: `notes:<repo>`, `conventions:<scope>`, `repo:<name>`, `skill:<name>` (or
  `store/skill`), or a bare name that is a repo entry or a skill (both is an error asking for a
  prefix). A repo opens its store's `bindings.toml` at the `[repos.<name>]` line; a skill its
  `SKILL.md`. Missing notes are created from `templates/notes.md`; a missing conventions file is
  created empty, at the scope's first `always` entry under `conventions/` or else
  `conventions/<scope>.md`, with a note when no `always` lists it.
- **Editor**: `$EDITOR`, else `config.toml` `editor`, else a user error. The command may carry
  arguments (`code -w`). Line syntax by program name: `file:line` for `hx`/`helix`, `subl`, `zed`,
  `micro`; `+line file` for vi-likes, `nano`, `emacs`, `kak`; `--goto file:line` for VS Code-likes;
  any other editor gets the file alone. A non-zero editor exit is an error and nothing is changed.
- **Supervision after the editor exits** (`edit::review_changed_drafts`): every `[repos.*]` value
  still carrying `# kipi:draft` whose value differs from before the edit (as data: quoting, spacing
  and comments do not count) loses its marker. This runs whatever the target, so hand-editing
  `bindings.toml` through `kipi edit` of any entry reviews what changed. Then every store is loaded
  and resolved again; a failure is reported as the usual `error:`/`hint:` with exit 1, and the
  edited file is kept as written.
- **`edit` needs the stores to load first** (it resolves the target through them). Fixing a store
  that no longer loads is done in the editor directly; the error already names `file:line`.
- **`ignore` and `regex`** come from the §12 list (`ignore` added to kipi-core, `regex` to
  kipi-cli to compile the pattern).
## Stage 3: token counting with cache and `kipi tree` (#10)

- **Counts come from `tiktoken-rs`** (now a regular `kipi-core` dependency) with the `config.toml`
  `tokenizer` (default `cl100k_base`), counted as ordinary text (`<|endoftext|>` in a file is
  just characters). They are estimates: Claude's tokenizer is not public. The character ÷ 4
  estimate is gone; `sync`, `status` and doctor's K010 (notes size) all use the counter. Each
  skill description is counted on its own, so the always-on total is block + Σ descriptions.
- **The token cache** is `~/.cache/kipi/tokens.<encoding>.json` (`$KIPI_CACHE_DIR` overrides; one
  file per encoding, so switching encodings never mixes counts). A file is keyed by (path, mtime
  in ns, size) and is not read while its stamp holds (conventions in `tree`, notes in doctor).
  Generated text has no file of its own: a rendered block, a registry or a skill description is
  keyed by its SHA-256. The tokenizer is built only on a miss, which is what makes a warm run
  fast (building `cl100k_base` alone takes ~250 ms in a debug build).
- **The Stage 2 / #12 notes on the character estimate** (`sync` / `status` always-on size, K010's
  "6,000 characters") are superseded by the above.
- **Git's tracked files are cached too** (`~/.cache/kipi/tracked.json`), keyed by the stamp of
  each checkout's `.git/index`, the file `git ls-files` reads. Measured on the 50-repo fixture,
  one `git ls-files` per repo was ~110 ms of a 120 ms warm `tree` (release); everything else was
  ~10 ms. A checkout whose `.git` is a file (worktree, submodule) is always asked; one with no
  index tracks nothing. `$GIT_INDEX_FILE` is not honored. `apply::checkout` and `apply::run` take
  the cache explicitly, and `sync::run` takes both caches as `cache::Caches`; tests pass
  `TrackedCache::default()`, which remembers for the process only.
- **Caches never fail a command.** An unreadable, malformed or other-version cache file reads as
  empty. Saving is skipped when nothing was added. `Caches::save` saves each cache on its own, so a
  token-cache failure never keeps the tracked cache from being written, and returns one error per
  failed cache; each is a `-v` note on stderr. A map
  over 4,096 entries keeps only what the run used. Writes are atomic, so two concurrent runs at
  worst lose one run's additions.
- **Dry-run syncs diff repos in parallel** (one thread per core, `std::thread::scope`), since a
  cold run waits on `git` per repo; results keep plan order. A sync that writes stays sequential
  and stops at the first error, as before.
- **`tree` layout** follows §7: one `[store <name>  <path>  <git state>[, shares skills]]` header
  per mounted store (git state from `git::status`, the helper `kipi stores` uses: `no git`
  outside a git work tree, else `git: clean` / `git: uncommitted changes`, plus `, no remote`),
  then its scopes
  without `extends` at column 0, each followed by its own skills (`●`, `[scope]`), its registry
  (`registry (N repos, T tok)`, only on the `registry = true` scope), the scopes extending it,
  then its repos by name. Repos that resolve to no scope go under `(unscoped)`. Token counts line
  up in one right-hand column. Box drawing (`├─ └─ │`) becomes `|- `- |` when stdout is not a
  terminal; the markers `○ ● ✗ ✓ ·` print either way, as decided for `explain`.
- **A repo line** shows its always-on tokens (as `1.3k` from 1,000 up), its sync state with the
  `status` vocabulary (`✓ synced`, `✗ stale`, `✗ missing`, `· not cloned`, `✗ checkout gone`;
  the same dry-run diff, so `tree` and `status` never disagree), then `notes` and `draft summary`
  tags. Its `skills:` line (inherited `○`, explicit `●`, excluded `✗`, in the order the chain and
  the repo bind them so an exclusion shows where the skill would have been) appears when the repo
  binds or excludes skills itself, as events-pipeline does in the sketch, or under a `--repo` /
  `--skill` filter; its `related:` line (`✓ granted`, `✗ not granted`, `· not cloned`) whenever it
  has related repos.
- **A scope's always-on** is what every repo in it pays before its own section and skills: the
  chain's `always` files (counted as files, so the cache applies), the registry it renders, and
  the descriptions of the chain's skills. That is why `acme-ml` (one more skill) is above `acme`
  in the sketch, and why each repo's own figure is higher still (its this-repo section and skills).
- **Filters.** `--store <name>` keeps one mounted store (an unknown name is a user error with a
  did-you-mean). `--repo <entry>` keeps the path to that repo (its store, its scope chain with
  their skills and registry lines) and expands it. `--skill <s>` keeps repos whose effective skills
  include it, matched by name, or by store and name when qualified (`acme/dbt-conventions`), and
  expands their skills; an unknown skill is a user error (naming the store that has it when the
  qualifier is wrong), and a known one no repo uses prints `no repo uses skill <s>`. Filters
  combine; scopes and stores left empty by `--repo` / `--skill` are dropped.
- **Display flags** change the text only: `--budget` shows `tokens/budget` and an `over budget`
  tag; `--machine` shows each scope's `machine.toml [roots]` override in place of its declared
  roots and each repo's checkout (or `(not cloned here)`). `--json` always carries every field
  (docs/json.md, "Tree").
- **Performance fixture.** `kipi_test_support::many::many_repos(home, n)` lays out a home with a
  store (`bench`: a registry scope with conventions and four team scopes), `n` committed git
  checkouts and `config.toml` + `machine.toml`. The CLI test syncs 50 repos, warms the caches with
  one `tree`, then asserts the best of three runs is under 100 ms. Measured on an 8-core macOS
  laptop: ~14 ms release, ~22 ms debug (the test runs the debug binary); with both caches empty
  ~170 ms release. The first CI run measured 120 ms on ubuntu-latest (about 2.7× the laptop, with
  other tests running beside it), so two changes keep the margin:
  - **Dependencies build with `opt-level = 2` in the dev profile** (`[profile.dev.package."*"]`
    in the workspace `Cargo.toml`; kipi's own crates stay unoptimized). Debug-built SHA-256,
    minijinja and TOML parsing were half of a debug run (45 → 27 ms). The cost is a slower
    first build of the dependencies.
  - **`git::status` (shared with `kipi stores`) answers a directory with no `.git` in it or any
    ancestor without starting `git`** (−5 ms per store that is not under git).

## Stage 3: doctor filesystem/budget checks and `--fix` (#13)

- **Codes K016–K022**, in the issue's order: budget, checkout-gone, stray-link, stale-grant,
  foreign-link, missing-exclude, tracked-block. Levels as in §10. The #12 note that `--fix` is
  a user error is superseded.
- **The filesystem checks look only at synced checkouts** (cloned here, with a
  `.kipi/manifest.toml` that loads). A never-synced checkout is `kipi status`'s `missing`, not a
  doctor problem; a malformed manifest is left to `sync`, which reports it with its hint.
  Orphaned checkouts (entries no mounted store has) are left to `sync --all`, which cleans them.
- **Symlinks are looked for in the adapters' skill directories only** (`.claude/skills/`,
  `.agents/skills/`), the only place kipi links. Walking a whole checkout would flag the user's
  own symlinks (build outputs, vendored trees) that kipi has no opinion on.
- **One finding per link.** kipi's own link (in the manifest) that is broken or points outside
  every mounted store is K018 with a re-sync fix. A link kipi did not create is K018 when broken
  (fix: remove it, a broken link holds nothing) or outside every store (no fix: "anything on disk
  that kipi did not write is never touched", §8, outranks §10's "remove" for a working link), and
  K020 otherwise. Store containment is checked on the path as written and resolved, since temp
  and home directories are often reached through a symlink (`/var` → `/private/var` on macOS).
- **K019 skips store directories the plan still grants.** A store's `repos/` holds notes only;
  git keeps no empty directories, so a store without notes usually has no `repos/`, and warning
  about it would never go away. Reported are entries the plan no longer grants (fix: re-sync)
  and checkouts that are gone (fix: K017's removal, then re-sync).
- **K021 compares against the manifest's `[exclude]` patterns**, what the last sync put there, not
  a fresh plan: a plan that changed since (another adapter) is a stale sync (`status`), not a
  missing exclude, and recomputing would misjudge repos synced with `--allow-tracked`. A pattern
  anywhere in the file counts, inside kipi's section or not.
- **K016's contributors** are each conventions file, the registry (rendered as a registry file),
  the rest of the block ("this-repo section": headings, related repos, lookup lines) and each
  skill description; the three largest are named. They are estimates like the total.
- **`--fix` order**: config edits (K015 `remote`, K017 `machine.toml` lines, comment-preserving),
  broken-link removal (`apply::remove_link`, only while the link still points where doctor saw
  it), then a re-sync of the named repos through `sync::run` on a plan resolved after the edits,
  so a deleted checkout's grant is dropped in the same run. The whole run holds the sync lock.
  Re-syncing uses doctor's prepared plan, so references reported as errors stay out of it (the
  same plan doctor checked). Doctor then runs again; the output lists `fixed: …` lines, then the
  remaining findings, and exits 2 if errors remain. `--fix --dry-run` prints `would …` lines and
  writes nothing; `--json` adds `fixed` and `skipped` arrays.
- **`fixed` lists only fixes that ran.** A fix that did not run goes in `skipped` with its
  reason, in the text and in `--json`: a broken link that changed since doctor looked, or a
  re-sync of a repo that has nothing left to sync after the config edits (no longer cloned,
  checkout gone, entry gone). It is a separate list rather than a flag on `fixed`, so a
  script reading `fixed` never mistakes a sync that did not happen for a repair. Plain `doctor --dry-run` stays accepted and
  writes nothing, as before.
- **CLI doctor tests run on an onboarded home** (checkouts laid out and scanned). The example
  `machine.toml` points at `/Volumes/...` and `/srv/...`, which do not exist in a test home, so
  every line would now be K017; the example file itself is unchanged and still valid.

## Stage 4: release engineering, docs and completions (#18)

- **cargo-dist config lives in `dist-workspace.toml`**, dist 0.32's default location, with
  `[profile.dist]` (release + thin LTO) in the root `Cargo.toml`. Targets: `x86_64`/`aarch64`
  `unknown-linux-musl` and `x86_64`/`aarch64` `apple-darwin`, each built natively on its own
  runner. Installers: `shell` and `homebrew`; checksums `sha256`; the shell installer writes to
  `~/.cargo/bin` and installs no updater (kipi has no network code and should not grow a
  self-updater by way of its installer). The formula is named `kipi`
  (`[package.metadata.dist] formula`), not after the `kipi-cli` package; archives keep the
  package name (`kipi-cli-<target>.tar.xz`). The tap is `tap = "martin-conur/homebrew-tap"` in
  `dist-workspace.toml`, so changing it is a one-line edit plus `dist generate`; the push uses the
  `HOMEBREW_TAP_TOKEN` secret. Maintainer setup: `docs/release.md`.
- **The PR dry run uploads every artifact and runs only for release-config PRs.**
  `pr-run-mode = "upload"` builds all four targets plus installers on a pull request and attaches
  them to the workflow run, publishing nothing. dist has no option to limit that to some paths, so
  `release.yml` carries one hand edit, a `paths` filter on `pull_request`
  (`dist-workspace.toml`, root and kipi-cli `Cargo.toml`, `rust-toolchain.toml`, `release.yml`),
  and `allow-dirty = ["ci"]` keeps `dist` from rejecting or overwriting it. The cost: regenerating
  is a manual procedure (documented in `docs/release.md`). Running on every PR was the
  alternative; four release builds per PR for a config that rarely changes was not worth it.
- **Fresh-machine E2E is a POSIX script, not a Rust test** (`scripts/fresh-machine-e2e.sh`), so it
  runs where no toolchain exists. CI builds the release binary for `x86_64-unknown-linux-musl`
  and `aarch64-apple-darwin` (`e2e-build`), then installs it into an empty `HOME` in a clean
  `debian:stable-slim` container (only `git` added) and on a fresh `macos-latest` runner that did
  not build it. The script creates two fake repos with remotes, writes a scope into the store the
  way a user would (there is no `add scope` command), and runs `init`, `scan`, `add skill`, `bind`,
  `sync --all` twice (the second must change nothing), `status`, `doctor` (must exit 0; drafted
  summaries are info) and `where`. It takes a binary or a dist `.tar.xz`, so it also checks a
  release archive locally.
- **Completions use clap_complete's dynamic engine** (`unstable-dynamic` feature of the §12
  `clap_complete`; no new crate, though it pulls in `clap_lex`, `shlex` and `is_executable`).
  Static scripts cannot complete names that live in the stores; the dynamic engine calls back into
  `kipi`, so repo names, aliases, skills (and `<store>/<skill>` for sharing stores), scopes, store
  names and `kipi edit`'s `notes:`/`conventions:` targets complete in bash, zsh, fish, elvish and
  powershell. `kipi completions <shell>` prints the registration script; the completion request
  itself is `KIPI_COMPLETE=<shell> kipi -- <words>` (the `KIPI_` prefix, not clap's default
  `COMPLETE`), handled first thing in `main`. The feature is unstable upstream, so the
  registration script must be loaded at shell startup (`eval "$(kipi completions bash)"`), never
  saved to a file: it then always matches the installed binary. Candidates load the configured
  stores only (no `resolve`, no `machine.toml`), are silent on any error, and ignore a `--store`
  typed on the same line (the engine gives candidate functions no access to parsed flags).
- **The book's source is `docs/` itself** (`src = "."`, output in `target/book`), so existing
  links to `docs/adapters.md`, `docs/doctor.md`, `docs/json.md` and `docs/decisions.md` stay valid.
  CI builds it with mdBook 0.5.4 (`create-missing = false`, so a broken `SUMMARY.md` entry fails).
- **Config-reference completeness reads the serde model.** `config::schema::keys(file)` asks each
  `Raw*` struct's derived `Deserialize` for its field list (a tiny introspecting deserializer that
  records `deserialize_struct`'s `fields` and stops); each loader composes the dotted paths
  (`scopes.<scope>.extends`). `tests/config_reference.rs` then requires the key column of each
  `## <file>` table in `docs/config-reference.md` to equal that set, both ways: a new field fails
  until documented, and a row for a removed key fails too.
- **Codex rows in `docs/adapters.md`:** `.agents/skills` (and symlinked skill folders) verified
  against the current skills page; the "reads outside the workspace" behaviour is not stated in
  OpenAI's docs and is marked unverified there, with what the docs do say.
- **Leak guard allows `/home/linuxbrew`.** The generated `release.yml` puts Homebrew's Linux
  prefix on `PATH` in the formula job. It is a fixed system account, not a person, so it joins
  the placeholder names (`PLACEHOLDER_USERS`) instead of a hand edit to generated code.
- **`release.yml` actions are pinned to commit SHAs** like `ci.yml`'s (review of #42): it holds
  `contents: write` and `HOMEBREW_TAP_TOKEN`. That is a second hand edit kept by
  `allow-dirty = ["ci"]`; `tests/workflow_pins.rs` in kipi-test-support fails on any `uses:` in
  `.github/workflows/` without a 40-hex SHA and a `# vX.Y.Z` comment, so a regeneration that
  forgets to re-pin fails `cargo test`. A test rather than a CI-only step, so it fails locally too.
- **The macOS E2E runs on `macos-14`**, an Apple Silicon image, not `macos-latest`: it builds and
  runs the `aarch64-apple-darwin` binary, and the label pins the architecture.

## Stage 4: public release 0.1.0 (#21)

- **No Homebrew in 0.1.0** (maintainer decision). `dist-workspace.toml` drops the `homebrew`
  installer, the `tap` and the `homebrew` publish job, and `crates/kipi-cli/Cargo.toml` drops
  `[package.metadata.dist] formula = "kipi"`: that key alone makes `dist plan` emit `kipi.rb`
  even with `installers = ["shell"]`. `release.yml` was regenerated and its two hand edits (SHA
  pins, PR `paths` filter) re-applied; the `publish-homebrew-formula` job and the
  `HOMEBREW_TAP_TOKEN` secret are gone. Install docs say "shell installer, prebuilt archive or
  build from source". Roadmap §14's Stage 4 acceptance reads "shell installer →" instead of
  `brew install kipi`. `docs/release.md` keeps the steps to add Homebrew back. The leak guard
  still allows `/home/linuxbrew`, since the formula job will bring that path back.
- **§6 gains the not-cloned placement rule** from "Stage 1: resolution engine (#3)" as rule 3,
  so the published spec matches the code; `personal` / unscoped becomes rule 4. Earlier entries
  in this file that say "rule 3" for the `personal` fallback predate the renumbering. Each §15
  open decision now names its decision and points at its entry here.
- **The history leak audit is a test.** `leak_guard::scan_history` runs the same scanner as the
  tree scan over every blob and commit message reachable from any ref except the stash (old
  copies of `.leak-denylist` skipped). `tests/leak_guard.rs::history_has_no_leaks` is
  `#[ignore]`d locally, because a leak already in history cannot be fixed by a commit, and runs
  in CI's `leak guard (history)` job with `fetch-depth: 0`, the forge's `refs/pull/*` fetched,
  and the `KIPI_LEAK_DENYLIST` secret.
- **Denylist additions are generic only:** more credential shapes (Anthropic, OpenAI, Google API
  keys, Slack webhooks, GitLab tokens), personal mailbox domains, macOS per-user temp dirs
  (`/var/folders/xx/<id>/`), and company chat/SSO tenant hosts. Employer names stay in the
  secret.
- **The README demo is reproducible.** `docs/demo/demo.tape` (vhs) sources
  `docs/demo/setup.sh`, which copies both example stores into a throwaway home at the fixed
  path `/tmp/kipi-demo` (so recorded paths do not vary per run, and never show a real home) and
  creates empty git repos for their entries. The tape writes `docs/demo/demo.gif` and the
  `kipi tree` screenshot `docs/demo/tree.png`. The setup refuses to clear `/tmp/kipi-demo` unless
  it carries its `.kipi-demo` marker.
- **Release notes come from `CHANGELOG.md`.** cargo-dist reads the section for the tagged
  version (Keep a Changelog headings, `## [0.1.0]`) into the GitHub Release body.
- **Code of conduct by reference.** `CODE_OF_CONDUCT.md` adopts the Contributor Covenant 2.1 by
  link rather than copying its text, and routes reports to the maintainer privately.
- **Help text.** `sync --agents` names its values and delimiter, `tree --budget` says it shows
  tokens against the budget, and `edit`'s argument lists the target forms.
