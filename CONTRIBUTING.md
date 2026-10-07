# Contributing to kipi

Thanks for helping. kipi is a small personal tool with a written spec: [`roadmap.md`](roadmap.md)
decides behaviour, and where it leaves a choice open the answer is recorded in
[`docs/decisions.md`](docs/decisions.md). If a change needs a decision the spec does not make,
add an entry there in the same PR.

## Dev setup

- Rust: the toolchain is pinned in `rust-toolchain.toml` (1.92, with rustfmt and clippy);
  `rustup` picks it up automatically.
- `git` on `PATH` (tests create real throwaway repos).
- Optional tools, each used by one check:

  ```bash
  cargo install cargo-insta     # review snapshot changes
  cargo install cargo-deny      # licenses and banned crates
  cargo install mdbook          # build the docs
  cargo install cargo-llvm-cov  # resolve.rs coverage
  cargo install cargo-dist      # release config (`dist plan`)
  brew install vhs              # re-record the README demo (docs/demo/demo.tape)
  ```

Run the CLI from a checkout with `cargo run -p kipi-cli -- --help`. Point it at the example
stores without touching your own config:

```bash
KIPI_CONFIG_DIR=examples cargo run -p kipi-cli -- \
  --store examples/personal-store --store examples/acme-store tree
```

## Before you open a PR

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo deny check
```

CI runs the same on Linux and macOS, builds the book (`mdbook build docs`) and runs the
fresh-machine E2E (`scripts/fresh-machine-e2e.sh target/debug/kipi`), which you can also run
locally: it uses a throwaway `HOME`.

## Rules the tests enforce

- **No organization-specific names, paths or examples** anywhere: code, docs, fixtures, tests.
  Examples use `acme`, `~/work/acme`, `billing-api`, `events-pipeline`, `ml-platform`, and
  personal repos under `~/code`. Write `~/...`, never an absolute home path. The leak guard
  (`cargo test -p kipi-test-support --test leak_guard`) scans every tracked file against
  `.leak-denylist`; put private patterns in `$KIPI_LEAK_DENYLIST` (newline-separated regexes),
  never in a commit. Tests that need a home path build it at runtime
  (`format!("/{}/{}", "Users", name)`).
- **No network, libgit2 or SQLite crates** (`deny.toml`). New dependencies come from the
  `[workspace.dependencies]` list in `Cargo.toml`; anything else needs a `docs/decisions.md`
  entry first.
- **Every user-facing error has a hint.**
- **Adapters stay under 100 lines each** (`tests/adapters.rs`).
- **The example stores stay valid**, alone and mounted together (`tests/examples.rs`).
- **Every workflow `uses:` is pinned to a commit SHA** with a version comment
  (`tests/workflow_pins.rs`).
- **`docs/config-reference.md` documents exactly the keys the serde model accepts**
  (`tests/config_reference.rs`).

## Test layers

| Layer | Where | Run |
| --- | --- | --- |
| Resolution: unit + property tests (pure, no I/O) | `crates/kipi-core/tests/resolve.rs`, `resolve_props.rs` | `cargo test -p kipi-core --test resolve --test resolve_props` |
| Rendering: insta snapshots + splice properties | `crates/kipi-core/tests/render.rs`, `splice_props.rs` | `cargo test -p kipi-core --test render --test splice_props` |
| Apply engine: tempdir integration | `crates/kipi-core/tests/apply.rs` | `cargo test -p kipi-core --test apply` |
| Doctor: one fixture per check code | `crates/kipi-core/tests/doctor.rs` | `cargo test -p kipi-core --test doctor` |
| CLI: insta snapshots of real runs | `crates/kipi-cli/tests/cli.rs` | `cargo test -p kipi-cli --test cli` |
| Leak guard, workflow pins, example stores | `crates/kipi-test-support/tests/` | `cargo test -p kipi-test-support` |
| Fresh machine, end to end | `scripts/fresh-machine-e2e.sh` | `scripts/fresh-machine-e2e.sh target/debug/kipi` |

Shared fixtures live in `kipi-test-support`: `stores.rs` copies the example stores, `repos.rs`
makes fake git repos, `builders.rs` builds in-memory stores for resolve tests.

## Snapshots (`cargo insta review`)

Rendering and CLI output are checked with [insta](https://insta.rs) snapshots under
`crates/*/tests/snapshots/`. When a change alters output on purpose:

```bash
cargo test                # failing snapshots write .snap.new files
cargo insta review        # inspect each diff; accept (a) or reject (r)
cargo test                # green again
```

Commit the updated `.snap` files with the change and never commit `.snap.new`. If a snapshot
changes and you did not expect it to, that is a bug, not a snapshot to accept.

## Updating an adapter's vendor paths

Agents move their files around. Each adapter (`crates/kipi-core/src/adapters/<id>.rs`) keeps
every vendor path and settings key in one constant block at the top, and
[`docs/adapters.md`](docs/adapters.md) records each one with its source URL and the date it was
checked. To update one:

1. Check the vendor's current docs. Quote the sentence that states the path or key.
2. Change the constant in the adapter. Keep the file under 100 lines.
3. Update the matching row in `docs/adapters.md`: the new value, the source URL and the date
   checked. If the docs do not state a behaviour, say so and mark it unverified.
4. `cargo test`, then `cargo insta review` for the rendering and CLI snapshots the change
   moves.
5. If the change alters what kipi writes into existing repos (for example a renamed skills
   directory), note in `CHANGELOG.md` that users must run `kipi sync --all`. The manifest
   removes the old entries.

Before each release, re-check every row in `docs/adapters.md`.

## Docs

The docs are an mdBook (`docs/`, `mdbook build docs`). If a change is user-visible, update the
page that describes it, plus `CHANGELOG.md` under "Unreleased".

## Recording the demo

The README GIF and tree screenshot come from `docs/demo/demo.tape`, run against copies of the
example stores in a throwaway home at `/tmp/kipi-demo`:

```bash
vhs docs/demo/demo.tape   # writes docs/demo/demo.gif and docs/demo/tree.png
```

## Releasing

See [`docs/release.md`](docs/release.md).

## Conduct

This project follows the [Code of Conduct](CODE_OF_CONDUCT.md).
