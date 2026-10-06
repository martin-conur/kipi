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
| `kipi.rb` | Homebrew formula, also pushed to the tap |
| `source.tar.gz` | Source archive |

Each target builds natively on its own GitHub runner (`ubuntu-22.04`, `ubuntu-22.04-arm`,
`macos-15-intel`, `macos-14`) with the `dist` profile (`release` + thin LTO).

## Dry run

- **Locally:** `dist plan` lists every artifact; `dist build --artifacts=all --target <host>`
  builds this machine's archive, the installer and the formula into `target/distrib/`.
- **On a pull request:** the release workflow runs in `pr-run-mode = "upload"` — it builds every
  artifact on every target and uploads them to the workflow run, and publishes nothing. It runs
  only on PRs that touch release config (`dist-workspace.toml`, `Cargo.toml`,
  `crates/kipi-cli/Cargo.toml`, `rust-toolchain.toml` or `release.yml`).
- **Every PR** also runs the fresh-machine E2E (`scripts/fresh-machine-e2e.sh`): a release build
  installed into an empty home in a clean `debian:stable-slim` container and on a fresh macOS
  runner, then `init`, `scan`, `sync --all` and `doctor` on fake repos.

## One-time maintainer setup

Do these once, before the first release.

1. **Create the tap repository** `martin-conur/homebrew-tap` (public, with a default branch and
   any initial commit). Homebrew users then install with `brew install martin-conur/tap/kipi`.
   To publish to a different tap, change `tap` in `dist-workspace.toml` and run `dist generate`.
2. **Create a token for the tap.** A fine-grained personal access token with access to only the
   tap repository and the **Contents: Read and write** permission.
3. **Add it as a repository secret** of `martin-conur/kipi` named **`HOMEBREW_TAP_TOKEN`**
   (Settings → Secrets and variables → Actions). The `publish-homebrew-formula` job uses it to
   push `Formula/kipi.rb` to the tap; nothing else reads it.
4. **Actions permissions:** Settings → Actions → General → Workflow permissions must allow
   "Read and write" so the workflow's `GITHUB_TOKEN` can create the GitHub Release (the workflow
   requests `contents: write`).

The release itself needs no other secret: `GITHUB_TOKEN` creates the Release and uploads the
artifacts.

## Cutting a release

1. Bump `version` in the root `Cargo.toml` (`[workspace.package]`), commit, merge to `main`.
2. Tag and push: `git tag v0.1.0 && git push origin v0.1.0`.
3. The release workflow plans, builds all four targets, creates the GitHub Release and pushes the
   formula to the tap. Check: `brew install martin-conur/tap/kipi`, then `kipi --version`.

A tag with a pre-release suffix (`v0.2.0-rc.1`) is published as a pre-release.

## Changing the release config

Edit `dist-workspace.toml`, then run `dist generate` and commit the regenerated `release.yml`.

`release.yml` carries **one hand edit**: the `paths` filter on its `pull_request` trigger, so the
dry run only runs on PRs that touch release config. Because of it, `dist-workspace.toml` sets
`allow-dirty = ["ci"]`, which makes `dist generate` leave `release.yml` alone. To regenerate:
comment out `allow-dirty`, run `dist generate`, put `allow-dirty` back and re-apply the `paths`
filter (the comment above it in `release.yml` marks it). See [Decisions](decisions.md),
"Stage 4: release engineering".

Before each release, also re-check the vendor paths in [Adapters](adapters.md).
