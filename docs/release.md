# Releasing

kipi releases with [cargo-dist](https://axodotdev.github.io/cargo-dist/) (`dist`). The
configuration is `dist-workspace.toml` at the repo root; `dist generate` turns it into
`.github/workflows/release.yml`.

## What a release produces

For each tag `vX.Y.Z` the release workflow builds, checksums and attaches to a GitHub Release:

| Artifact | What |
| --- | --- |
| `kipi-cli-x86_64-unknown-linux-musl.tar.xz` | Static Linux binary (x86_64) |
| `kipi-cli-aarch64-unknown-linux-musl.tar.xz` | Static Linux binary (aarch64) |
| `kipi-cli-x86_64-apple-darwin.tar.xz` | macOS binary (Intel) |
| `kipi-cli-aarch64-apple-darwin.tar.xz` | macOS binary (Apple silicon) |
| `*.tar.xz.sha256`, `sha256.sum` | SHA-256 checksums |
| `kipi-cli-installer.sh` | Shell installer: picks the archive for the platform, verifies it, installs into `~/.cargo/bin` |
| `source.tar.gz` | Source archive |

Each target builds natively on its own GitHub runner (`ubuntu-22.04`, `ubuntu-22.04-arm`,
`macos-15-intel`, `macos-14`) with the `dist` profile (`release` + thin LTO).

## Dry run

- **Locally:** `dist plan` lists every artifact; `dist build --artifacts=all --target <host>`
  builds this machine's archive and the installer into `target/distrib/`.
- **On a pull request:** the release workflow runs in `pr-run-mode = "upload"` — it builds every
  artifact on every target and uploads them to the workflow run, and publishes nothing. It runs
  only on PRs that touch release config (`dist-workspace.toml`, `Cargo.toml`,
  `crates/kipi-cli/Cargo.toml`, `rust-toolchain.toml` or `release.yml`).
- **Every PR** also runs the fresh-machine E2E (`scripts/fresh-machine-e2e.sh`): a release build
  installed into an empty home in a clean `debian:stable-slim` container and on a fresh macOS
  runner (`macos-14`, Apple Silicon: the label is pinned rather than `macos-latest` so the
  `aarch64-apple-darwin` binary always matches the runner's architecture), then `init`, `scan`, `sync --all` and `doctor` on fake repos.

## One-time maintainer setup

Do this once, before the first release.

- **Actions permissions:** Settings → Actions → General → Workflow permissions must allow
  "Read and write" so the workflow's `GITHUB_TOKEN` can create the GitHub Release (the workflow
  requests `contents: write`).

The release needs no other secret: `GITHUB_TOKEN` creates the Release and uploads the artifacts.

## Homebrew (deferred)

0.1.0 ships without a Homebrew formula: install with the shell installer or from source. To add
Homebrew later, set `installers = ["shell", "homebrew"]`, `tap = "<owner>/homebrew-tap"` and
`publish-jobs = ["homebrew"]` in `dist-workspace.toml` (and `formula = "kipi"` under
`[package.metadata.dist]` in `crates/kipi-cli/Cargo.toml`, so the formula is not named after the
package), regenerate `release.yml` as below, create the public tap repository, and add a
fine-grained token with **Contents: Read and write** on only that repository as the
`HOMEBREW_TAP_TOKEN` repository secret. The generated formula job puts `/home/linuxbrew` on
`PATH`, which the leak guard already allows (`PLACEHOLDER_USERS`).

## Cutting a release

1. Bump `version` in the root `Cargo.toml` (`[workspace.package]`) and both `version` fields in
   its `[workspace.dependencies]`, move the `CHANGELOG.md` "Unreleased" entries under the new
   version, commit, merge to `main`.
2. Tag and push: `git tag v0.1.0 && git push origin v0.1.0`.
3. The release workflow plans, builds all four targets and creates the GitHub Release. Check on a
   macOS and a Linux machine: run the shell installer from [Install](install.md), then
   `kipi --version`, `kipi init`, `kipi scan`, `kipi sync --all`.

A tag with a pre-release suffix (`v0.2.0-rc.1`) is published as a pre-release.

## Changing the release config

Edit `dist-workspace.toml`, then regenerate `release.yml` and re-apply kipi's two hand edits.

`release.yml` carries **two hand edits** on top of what `dist generate` writes:

1. **Every `uses:` pinned to a full commit SHA** with a `# vX.Y.Z` comment, like `ci.yml`. This
   workflow holds `contents: write`, so a moved tag must not be able to
   change what it runs. `dist generate` writes mutable tags (`actions/checkout@v6`).
2. **A `paths` filter on the `pull_request` trigger**, so the dry run only runs on PRs that touch
   release config.

Because of them, `dist-workspace.toml` sets `allow-dirty = ["ci"]`, which makes `dist generate`
(and `dist plan`) leave `release.yml` alone. To regenerate:

1. Comment out `allow-dirty = ["ci"]` in `dist-workspace.toml` and run `dist generate`.
2. Put `allow-dirty = ["ci"]` back.
3. Re-apply the `paths` filter (copy it, with its comment, from the previous `release.yml` in
   `git diff`) and the comment block above `name: Release`.
4. Pin every `uses:`: for each `owner/repo@vN`, resolve the commit with
   `gh api repos/<owner>/<repo>/commits/vN --jq .sha` and the matching release with
   `gh api repos/<owner>/<repo>/releases --jq '.[].tag_name'`, then write
   `owner/repo@<sha> # vN.x.y`. (`pinact run .github/workflows/release.yml` does the same.)
5. `cargo test -p kipi-test-support --test workflow_pins` must pass: it fails on any `uses:` in
   `.github/workflows/` that is not a 40-hex SHA with a version comment, so a forgotten step 4
   fails `cargo test` locally and in CI.
6. Open a PR; the release dry run runs on it. Confirm it passes before merging.

See [Decisions](decisions.md), "Stage 4: release engineering".

Before each release, also re-check the vendor paths in [Adapters](adapters.md).
