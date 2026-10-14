# FAQ

## Why not cards?

A "repo card" would be a curated summary of each repo — its interfaces, invariants, how to call
it — generated or written once and copied into every repo that needs it. kipi deliberately does
not do this in v1:

- **Cards go stale; code does not.** A card is a second copy of knowledge that already lives in
  the repo. The registry line points the agent at the real repo, whose README, OpenAPI file and
  source are current by definition.
- **Cards cost context every turn.** A card for each related repo in the always-on block would
  blow the budget quickly. A registry line costs ~50 tokens; reading the real repo costs nothing
  until the agent needs it.
- **Agents are good at reading code.** Given a path and a one-line pointer ("OpenAPI in
  `api/openapi.yaml`"), the agent finds what it needs faster than a human can curate it.
- **What the code cannot say goes in notes.** Invariants and gotchas belong in a short,
  hand-written `notes.md`, which the registry line points at and which loads only on demand.

Cards (and interface generators for OpenAPI, Cargo, dbt and the like) remain possible future
work — for repos that are not cloned locally, or too large to read — and the seam is in place.

## Why not MCP?

- **Files are the interface every agent already has.** Claude Code, Codex CLI, Cursor and others
  all load `CLAUDE.md` / `AGENTS.md`, scan skill directories and read files by path. kipi arranges
  files; there is nothing to install in the agent, nothing to configure per tool and nothing that
  breaks when a tool's MCP support changes.
- **No server to run.** No daemon, no port, no process to keep alive across sessions, no database
  to migrate. `kipi` runs when you run it and is gone.
- **The cost model is better.** An MCP server's tool descriptions are paid every turn whether or
  not the agent uses them. The registry is small, budgeted and capped; skills cost only their
  description until used; everything else is on demand.
- **Shell covers lookup.** `kipi where <alias>` and `kipi grep <term> --repos` give an agent
  cross-repo lookup through the shell channel it already has.

A retrieval server stays possible later: `kipi where` and `kipi grep` already expose the
operations one would need.

## Does kipi send anything anywhere?

Not on its own. kipi has no network code (CI bans the crates) and never reads credentials.
Stores are plain directories you sync however you like — or not at all. The one exception is
yours to run: for a store that is a git repo, `kipi pull` and `kipi push` run the `git` binary
in that store, against the remote you configured, with git's own credentials.

## Will kipi touch my CLAUDE.md?

Only between its markers. Everything outside the generated block is yours. If `CLAUDE.md` is
tracked in git, kipi writes the block to the untracked `CLAUDE.local.md` instead; a tracked
`AGENTS.md` gets no block and a notice, since those agents have no additive untracked file. Pass
`--allow-tracked` to write into tracked files anyway (see [Decisions](decisions.md)). Every file kipi writes into a repo is recorded in
`.kipi/manifest.toml` and removed again when it is no longer wanted.

## How do I keep work and personal knowledge apart?

Use two stores: a personal one mounted everywhere and a work one mounted only on the work laptop.
Nothing crosses stores except skills a store explicitly shares, and `doctor` reports a work repo
(by `remote_patterns`) recorded in the personal store. `kipi scan` never adds such a repo to the
personal store in the first place: it reports it with the command that adds it to the work store.
That check needs the work store mounted, so on a machine that holds work checkouts, keep the
personal roots narrow (`~/code`, not all of `~/gits`). See [Concepts](concepts.md#scopes-and-the-leak-rule).

## Windows?

Not in v1: kipi relies on symlinks for skill directories.
