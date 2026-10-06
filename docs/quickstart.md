# Quickstart: ten repos

This walks through a typical setup: six work repos under `~/work/acme` and four personal repos
under `~/code`, kept in **two stores** so work knowledge never reaches a personal repo. Every
command that writes takes `--dry-run`; use it whenever you want to see a change first.

```text
~/work/acme/billing-api        ~/code/side-tool
~/work/acme/acme-api           ~/code/api
~/work/acme/web-app            ~/code/dotfiles
~/work/acme/infra              ~/code/blog
~/work/acme/ml/events-pipeline
~/work/acme/ml/ml-platform
```

## 1. Create the two stores

A store is a plain directory. The personal one lives at the default location; the work one lives
next to the work repos (keep it local, or in storage your employer approves).

```bash
kipi init                                          # ~/.agents/store, named `personal`
kipi init --store ~/work/.kipi-acme --name acme    # the work store
kipi stores                                        # both are now mounted
```

`init` writes a commented `bindings.toml` skeleton into each store and lists the store in
`~/.config/kipi/config.toml`. Add `--git` to `git init` a store; kipi itself never needs git for
stores and has no sync transport — sync each store however its trust boundary allows.

## 2. Declare the scopes

Scopes are configuration: open each `bindings.toml` (`kipi edit` needs a repo or skill, so use
your editor) and add them.

```toml
# ~/work/.kipi-acme/bindings.toml
[scopes.acme]
roots = ["~/work/acme"]
always = ["conventions/acme.md"]   # optional: short always-on text for every acme repo
registry = true
remote_patterns = ["github.com/acme-org/"]

[scopes.acme-ml]
extends = "acme"
roots = ["~/work/acme/ml"]
```

```toml
# ~/.agents/store/bindings.toml
[scopes.personal]
roots = ["~/code"]
registry = true
```

Nested roots are fine: `events-pipeline` sits under both roots and lands in `acme-ml`, the deepest.

## 3. Scan

```bash
kipi scan --dry-run   # what would be recorded
kipi scan
```

`scan` walks the scope roots for git checkouts, records where each one lives in
`~/.config/kipi/machine.toml` (this machine only, never in a store), and adds a
`[repos.<name>]` entry to the store that owns its scope, with the normalized `remote`, a drafted
`summary` (from `Cargo.toml` / `pyproject.toml` / `package.json` or the README) and drafted
`aliases`. Drafted values carry a `# kipi:draft` marker.

Two repos named `api` — `~/work/acme/acme-api` is fine, but if both were cloned as `api`, the
second entry would be named `<owner>-api` and matched by its remote.

## 4. Review the registry

The registry is only as good as its summaries. Review the drafts:

```bash
kipi tree             # stores → scopes → repos, drafts flagged
kipi edit billing-api # opens bindings.toml at the entry; saving marks it reviewed
```

Or edit the TOML directly and delete the `# kipi:draft` marker. kipi never rewrites a value once a
human has touched it. For knowledge the code cannot tell — invariants, gotchas — add notes:
`kipi edit notes:billing-api`.

## 5. Skills and relations

```bash
kipi add skill code-review --scope acme             # from the SKILL.md template
kipi add skill rust-style --scope personal --from ~/.claude/skills/rust-style   # import one
kipi bind events-pipeline --related billing-api ml-platform
kipi explain events-pipeline                        # where every decision comes from
```

A `related` repo is listed in the repo's own section and pre-granted for reads. A relation that
crosses scopes (`events-pipeline` → `side-tool`) is refused: that is the leak rule.

To use a personal skill at work, set `share_skills = true` in the personal store's `[store]` and
reference it as `personal/<skill>` from the acme scope. Nothing else crosses stores.

## 6. Sync

```bash
kipi sync --all --dry-run
kipi sync --all
kipi status
```

Each of the ten repos now has a generated block in `CLAUDE.md` and `AGENTS.md` (conventions,
registry, this-repo section), skill links in `.claude/skills/` and `.agents/skills/`, and a read
grant in `.claude/settings.local.json`. Everything kipi wrote is recorded in `.kipi/manifest.toml`
and listed in `.git/info/exclude`, so your tracked files are untouched. Running `sync` again is a
no-op; after editing a store, run `kipi sync --all` again.

## 7. Check

```bash
kipi doctor          # exits 2 on an error-level finding
kipi doctor --fix    # applies the fixes marked safe
```

`doctor` catches misclassified repos (a work remote recorded in the personal store), alias
collisions, budgets, stale links and grants, and drafts still waiting for review. See
[Doctor checks](doctor.md).

## A second machine

Get the stores onto it (any way you like), then:

```bash
kipi init --store ~/.agents/store           # mounts an existing store
kipi init --store ~/work/.kipi-acme         # only on the work laptop
kipi scan && kipi sync --all
```

Repos are matched by remote, so a different clone path or directory name is still recognized.
