@.claude/gh-workflow.md

# kipi
Rust workspace: kipi-core (lib, no I/O beyond fs), kipi-cli (bin), kipi-test-support.
Spec: roadmap.md. Rules: no network crates; no SQLite; adapters < 100 lines each; every user-facing error has a hint.
Never hardcode a home path; use dirs + machine.toml. No organization names anywhere (CI denylist).
Tests: unit for resolve, tempdir integration for apply, insta snapshots for rendering and CLI.
Run `cargo test && cargo clippy -D warnings` before claiming a task is done.
Example stores live in examples/acme-store and examples/personal-store and must stay valid.

The spec (`roadmap.md`) wins on any ambiguity; otherwise record the decision in `docs/decisions.md`.

## Workspace layout

```
Cargo.toml                 # workspace; the §12 dependency set is pinned in [workspace.dependencies] only
rust-toolchain.toml        # pinned toolchain (rustfmt + clippy)
deny.toml                  # license allowlist; bans tokio, reqwest, hyper, git2, rusqlite (+ -sys crates)
.leak-denylist             # leak-guard regexes (generic only; private ones via env, see below)
crates/
  kipi-core/src/           # lib: model, config/, frontmatter, resolve, plan, adapters/, render/,
                           #      apply, manifest, json_merge, scan, doctor/, tokens, search, git,
                           #      error, templates (include_str! of templates/)
  kipi-cli/src/            # bin `kipi`: main.rs, commands/ (one file per subcommand), output/
  kipi-test-support/src/   # stores.rs (copy example stores), repos.rs (fake git repos),
                           # builders.rs (in-memory stores/machines for resolve tests),
                           # leak_guard.rs (scanner); tests/ holds the repo-wide leak test
templates/                 # SKILL.md, notes.md, bindings.toml skeletons
examples/
  acme-store/              # work store: scopes acme + acme-ml, 6 repos, 4 skills
  personal-store/          # personal store: scope personal, 4 repos, 2 skills, share_skills = true
  machine.toml config.toml # example per-machine config (placeholder paths)
docs/decisions.md          # decisions the spec leaves open
```

New dependencies come from the §12 list via `<dep>.workspace = true`. Anything outside that list
needs an entry in `docs/decisions.md` first.

## Command cheat sheet

```bash
cargo fmt --all                                            # format
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace                                     # all tests, incl. leak guard + example stores
cargo test -p kipi-test-support --test leak_guard          # leak guard only
cargo test -p kipi-core --test resolve --test resolve_props  # resolution unit + property tests
cargo llvm-cov -p kipi-core --test resolve --test resolve_props  # resolve.rs coverage (cargo install cargo-llvm-cov)
cargo deny check                                           # licenses + banned crates (cargo install cargo-deny)
cargo run -p kipi-cli -- --help                            # run the CLI
```

CI (`.github/workflows/ci.yml`) runs fmt, clippy, test and `cargo deny check` on ubuntu-latest and
macos-latest, plus a report-only nightly branch-coverage job for `resolve.rs`.

## Leak guard

`crates/kipi-test-support/tests/leak_guard.rs` scans every file not ignored by `.gitignore` for the
patterns in `.leak-denylist` and for absolute home paths (`/Users/<name>`, `/home/<name>`). Write
`~/...` in docs and examples. Private patterns (a real employer's name) go in `$KIPI_LEAK_DENYLIST`
(newline-separated regexes; CI reads the repository secret of the same name) or in a file named by
`$KIPI_LEAK_DENYLIST_FILE`; never commit them. Tests that need a home path build it at runtime
(`format!("/{}/{}", "Users", name)`) so the source itself stays clean.
