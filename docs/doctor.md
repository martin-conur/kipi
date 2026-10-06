# Doctor checks

`kipi doctor [repo]` runs every check in roadmap §10 and prints one finding per problem,
each with a hint. It exits 2 when any finding is an **error**; warnings and info leave the
exit code at 0. `--json` prints the same findings (shape below). `--fix` applies the fixes
marked safe (see [Fixing](#fixing)).

Codes are stable: a check keeps its code for good, and a removed check's code is never
reused. With a `repo` argument only findings about that repo entry are printed; findings
about a skill or a store file (K005 in a scope, K006 in `bindings.toml` or conventions,
K009, K011) are left out.

The filesystem checks K018–K022 look only at **synced** checkouts: repos cloned here
(their `machine.toml [repos]` path is a directory) that hold a `.kipi/manifest.toml`. A
checkout kipi never synced is not kipi's business; `kipi status` reports it as `missing`.

## How doctor runs

Some problems stop `kipi resolve` outright: a reference into another store, a skill that
does not exist, a `related` repo outside the scope chain. Doctor reports each of them and
removes the offending reference, so the remaining checks still run on everything else.
Other configuration errors with no check of their own (a cycle in `extends`, an unknown
`scope`, a malformed file) are printed as the error they are, and doctor exits 1.

Checkout remotes (K001, K013, K014, K015) are read with `git config --get
remote.origin.url` in every `machine.toml [repos]` checkout that exists. A repo not cloned
here is identified by its entry's `remote`.

## K001 remote-scope-mismatch

**error.** The repo's remote (its checkout's origin, else the entry's `remote`) matches a
scope's `remote_patterns`, but the repo resolved to a scope outside that scope's chain:
e.g. a work repo cloned under `~/code` and placed in `personal`. A pattern of the resolved
scope or one of its ancestors is fine. The hint prints the `scope = "…"` line to add.

## K002 root-not-ancestor

**warn.** A cloned repo with no explicit `scope` was placed in a scope none of whose roots
(after `machine.toml [roots]`) holds its checkout. This is the `personal` fallback catching
a repo that lives somewhere else. Set `scope` explicitly if that is right, or add a root.

## K003 scope-crossing

**error.** A `related` repo outside the repo's scope chain (same scope, an ancestor or a
descendant), or a granted path that crosses scopes or stores. Resolution never produces the
latter; doctor checks the plan anyway, and reports one as a bug.

## K004 alias-collision

**error.** Two repos of one registry claim the same alias or entry name (after
case-folding and whitespace normalization), so the agent cannot tell which is meant. One
finding per alias, naming every repo that claims it.

## K005 missing-skill

**error.** A `skills` or `exclude` entry names a skill that does not exist: a bare name not
in the store's `skills/`, a qualified `<store>/<skill>` whose store is not mounted, or one
the sharing store does not have.

## K006 home-path

**error.** A store's `bindings.toml`, a `repos/<name>/notes.md`, a `conventions/*.md` or a
scope's `always` file holds an absolute home path (`/Users/<name>`, `/home/<name>`, or this
machine's home directory). Stores move between machines; write `~/…`, and keep absolute
paths in `machine.toml`. One finding per file, at the first such line.

## K007 unscoped

**warn.** A repo with no scope: no explicit `scope`, no root holds its checkout, and its
store has no `personal` scope. It gets no conventions, skills or registry.

## K008 summary

**info.** A repo with no `summary` (the registry lists it by name only), or a summary still
marked `# kipi:draft`.

## K009 skill-description

**warn.** A skill whose `description` is missing (or blank) or longer than 1,024
characters. Agents read only the description to decide when to load a skill.

## K010 notes-size

**info.** A repo's `notes.md` estimated over 1,500 tokens (target: 500), counted with the
`config.toml` `tokenizer` (default `cl100k_base`).

## K011 registry-size

**info.** A registry listing more than 40 repos, which every repo of the scope carries on
every turn. Consider splitting the scope.

## K012 cross-store

**error.** A reference into another mounted store: `extends`, a repo's `scope`, `related`
or `exclude`, a bare skill name that only another store has, or a qualified skill of a store
that does not set `share_skills = true`. Nothing crosses stores except shared skills.

## K013 wrong-store

**error.** A repo whose remote matches a scope's `remote_patterns` in another store and none
in its own: e.g. a work repo recorded in the personal store. The hint names the store to
move the entry (and its notes) to.

## K014 checkout-mismatch

**warn.** A checkout whose origin differs from its entry's `remote`, or two
`machine.toml [repos]` checkouts matching the same entry by the rules `scan` uses (remote,
then directory name, then origin basename).

## K015 missing-remote

**info.** An entry without `remote` whose checkout has a network origin. Safe fix: fill
`remote` with the normalized origin (a comment-preserving edit of `bindings.toml`).

## K016 budget

**warn.** A repo's always-on text is over its `budget_tokens`: the generated block plus the
`description` of each effective skill, counted as `sync` and `status` count it (the
`config.toml` `tokenizer`, an estimate). The message names the three largest parts among
each conventions file, the registry, the this-repo section (the rest of the block) and each
skill description. No fix.

## K017 checkout-gone

**warn.** A `machine.toml [repos]` path that is no longer a directory. Safe fix: remove the
line. If the repo only moved, `kipi scan` records the new path instead.

## K018 stray-link

**warn.** A symlink in a synced checkout's skill directories (`.claude/skills/`,
`.agents/skills/`) pointing at a path that does not exist or lies outside every mounted
store. Safe fix: for a link kipi created (its manifest lists it), re-sync the repo, which
re-links or removes it; for a broken link kipi did not create, remove it. A working link of
the user's that points outside the stores is reported and left alone.

## K019 stale-grant

**warn.** A read-grant entry kipi merged into a settings file (its manifest lists it) that
points at a path that does not exist. Safe fix: re-sync the repo, which drops an entry the
plan no longer grants; a related checkout that is gone is first removed from `machine.toml`
(K017's fix), so its entry goes too. A store's `repos/` or `skills/` the plan still grants is
not reported: git keeps no empty directories, and the grant is harmless until it appears.

## K020 foreign-link

**info.** A symlink in a synced checkout's skill directories that kipi's manifest does not
list, pointing into a mounted store. kipi leaves it alone; the next sync adopts it if the
plan links the same skill there. (A foreign link that is broken or points outside the stores
is K018 instead; each link gets one finding.)

## K021 missing-exclude

**warn.** A pattern kipi keeps in a synced checkout's `.git/info/exclude` (as its manifest
records them) is no longer in the file, so git shows kipi's links, settings or manifest as
untracked files. Safe fix: re-sync the repo, which rewrites kipi's section.

## K022 tracked-block

**info.** A file holding kipi's generated block that git tracks (written with
`sync --allow-tracked`, or tracked afterwards), so the block shows in `git diff`. No fix:
keep it, or re-sync without `--allow-tracked` to move the block into an untracked side file.

## Fixing

`kipi doctor --fix` applies every safe fix of the findings (of one repo with `kipi doctor
<repo> --fix`), under the sync lock, in this order:

1. Config edits: `remote` into `bindings.toml` (K015), `machine.toml [repos]` lines removed
   (K017). Both keep comments and ordering.
2. Broken links kipi did not create are removed (K018), each only if it still points where
   it did when doctor looked.
3. The stores and `machine.toml` are loaded and resolved again, and each repo a fix names is
   re-synced through the apply engine (K018, K019, K021), exactly as `kipi sync <repo>`
   would, so nothing outside what kipi wrote is touched. References doctor reports as
   errors (K003, K005, K012) stay out of the plan it syncs.

Doctor then runs again and prints what was fixed (`fixed: …`) followed by the findings that
remain. Running `--fix` again finds nothing left to fix (`nothing to fix`). With
`--dry-run` it prints the fixes it would apply (`would …`) and changes nothing.

## JSON

```json
{
  "version": 1,
  "findings": [
    {
      "level": "error",
      "code": "K004",
      "check": "alias-collision",
      "repo": "web",
      "at": "work:bindings.toml:12",
      "message": "alias `front end` is claimed by `web` and `web-old` in registry `work`",
      "hint": "aliases (and entry names) must be unique within a registry; …",
      "fix": null
    }
  ],
  "summary": { "error": 1, "warn": 0, "info": 0 }
}
```

`fix` is `null` or one of

- `{ "kind": "fill-remote", "store": …, "repo": …, "remote": … }`
- `{ "kind": "remove-checkout", "entry": …, "path": … }`
- `{ "kind": "remove-link", "link": …, "target": … }`
- `{ "kind": "resync", "repo": … }`

With `--fix`, the document also has `"fixed"`: the fixes applied (an empty list with
`--dry-run` or when there was nothing to fix), in the same shape, before `findings` (which
are then the findings that remain).
