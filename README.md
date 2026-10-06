# kipi
kipi is a single-binary Rust CLI that manages skills and cross-repo knowledge for LLM coding agents (Claude Code, Codex CLI, Kiro, Cursor and any agent that reads SKILL.md / AGENTS.md) across many local git repositories, without an MCP server or a database.

Status: pre-release scaffold. The build spec is [`roadmap.md`](roadmap.md); example stores live in [`examples/`](examples/).

```bash
cargo run -p kipi-cli -- --help
```

Working today (Stage 1): `kipi resolve [repo]` prints the resolved plan (`--json` is stable, see
[`docs/json.md`](docs/json.md)) and `kipi explain <repo>` shows where every scope, skill, grant and
setting decision comes from. Try them on the example stores:

```bash
KIPI_CONFIG_DIR=examples cargo run -p kipi-cli -- \
  --store examples/personal-store --store examples/acme-store explain events-pipeline
```
