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
