<!-- Link the issue: Closes #N -->

## What and why

## Checklist

- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`
      and `cargo test --workspace` pass
- [ ] Snapshot changes reviewed with `cargo insta review` and intended
- [ ] User-visible change: docs updated and a `CHANGELOG.md` entry under "Unreleased"
- [ ] A choice the spec leaves open is recorded in `docs/decisions.md`
- [ ] No organization names or absolute home paths (the leak guard passes)
