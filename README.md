# kipi
kipi is a single-binary Rust CLI that manages skills and cross-repo knowledge for LLM coding agents (Claude Code, Codex CLI, Kiro, Cursor and any agent that reads SKILL.md / AGENTS.md) across many local git repositories, without an MCP server or a database.

Status: pre-release scaffold. The build spec is [`roadmap.md`](roadmap.md); example stores live in [`examples/`](examples/).

```bash
cargo run -p kipi-cli -- --help
```

Working today: `kipi resolve [repo]` prints the resolved plan (`--json` is stable, see
[`docs/json.md`](docs/json.md)) and `kipi explain <repo>` shows where every scope, skill, grant and
setting decision comes from. `kipi scan` records your checkouts in `machine.toml`, `kipi sync
[repo...] [--all]` writes the generated blocks, skill links, read grants and `.git/info/exclude`
entries into them (`--dry-run` shows the changes first), and `kipi status [repo]` says which repos
are synced, stale or missing. `kipi init` creates (or mounts) a store and `kipi stores` lists or
unmounts them; `kipi add skill|repo`, `kipi bind` and `kipi unbind` edit `bindings.toml` while
keeping your comments and layout, and refuse any edit that would break resolution (such as a
relation crossing scopes). Every writer takes `--dry-run`. For agents (and you): `kipi where <alias>` resolves an alias to a repo's summary, checkout and
notes, `kipi grep <pattern> [--repos]` searches the conventions, notes and skills (and with
`--repos` the checkouts) of the current repo's scope chain, and `kipi edit
<repo|skill|notes:<repo>|conventions:<scope>>` opens the right file in `$EDITOR`, then validates it
and marks edited drafts reviewed. Try the read-only commands on the example stores:

```bash
KIPI_CONFIG_DIR=examples cargo run -p kipi-cli -- \
  --store examples/personal-store --store examples/acme-store explain events-pipeline
```
