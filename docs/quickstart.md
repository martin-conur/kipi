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

A scope says which directories hold its repos. Declare each one with `add scope`; it edits the
store's `bindings.toml` in place, keeping its comments, and checks the result before writing:

```bash
kipi add scope acme --store acme --root ~/work/acme
kipi add scope acme-ml --extends acme --root ~/work/acme/ml   # goes in acme's store
kipi add scope personal --store personal --root ~/code
```

Each scope gets a registry (the "Known repos" list in every repo's block) unless you pass
`--no-registry`, or it extends a scope that already has one: `acme-ml`'s repos are listed in
`acme`'s registry. `--extends` names a scope of the same store (nothing crosses stores); with it,
`--store` can be left out. `--skill <name>...` binds skills to the scope as it is created. With a
single store, `kipi init --scope personal --root ~/code` creates the store and its first scope in
one step.

The result, which you can also write by hand (`remote_patterns` has no flag yet):

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

Change a scope later with `kipi scope`, which edits the table in place the same way:

```bash
kipi scope personal --no-registry            # or --registry
kipi scope acme --add-root ~/src/acme --remove-root ~/work/acme
kipi scope acme --always conventions/acme.md --skill code-review --remove-skill old-skill
kipi scope acme-ml --no-extends              # or --extends <scope>
```

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

A checkout under a personal root whose remote matches another store's `remote_patterns` (an
`acme-org` repo cloned into `~/code`) is never added to the personal store: `scan` reports it
and names the fix, adding its directory to an acme root or running `kipi scan <path> --store acme
--scope acme`. That check only works while the work store is mounted, so on a machine that also
holds work checkouts, scope the personal roots narrowly (`~/code`, not all of `~/gits`) rather
than relying on it.

Two repos named `api` — `~/work/acme/acme-api` is fine, but if both were cloned as `api`, the
second entry would be named `<owner>-api` and matched by its remote.

A repo outside every root can be registered by hand: `kipi add repo <name> --scope <s>
[--path <checkout>]`. An entry without a checkout here shows as `· not cloned`; once you clone it,
`kipi add repo <name> --path <checkout>` records it. `kipi remove repo <name>` deletes an entry,
its `machine.toml` line and what kipi wrote into its checkout (`--notes` also deletes its
`notes.md`); it is refused while another entry lists it in `related`.

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

## Worktrees

kipi's files are untracked, so `git worktree add` makes a working tree without them. A linked
worktree is the same repo entry as its checkout, and `kipi sync <repo>` / `kipi sync --all` sync
it too. Inside one, a bare `kipi sync` syncs just that working tree. To have new worktrees
synced the moment they are made, install the git hook once per repo:

```bash
kipi hook install --all       # a post-checkout hook in each repo; one covers all its worktrees
git -C ~/work/acme/billing-api worktree add ../billing-api-feat   # already has the block
kipi hook uninstall --all     # removes it, putting back any hook it had replaced
```

A hook already in `.git/hooks/post-checkout` keeps running: kipi's hook calls it first. Removing
a worktree removes kipi's files with it; nothing is left for `doctor` to report.

## A second machine

Get the stores onto it (any way you like), then:

```bash
kipi init --store ~/.agents/store           # mounts an existing store
kipi init --store ~/work/.kipi-acme         # only on the work laptop
kipi scan && kipi sync --all
```

Repos are matched by remote, so a different clone path or directory name is still recognized.
