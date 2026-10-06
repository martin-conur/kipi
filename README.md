# kipi
kipi is a single-binary Rust CLI that manages skills and cross-repo knowledge for LLM coding agents (Claude Code, Codex CLI, Kiro, Cursor and any agent that reads SKILL.md / AGENTS.md) across many local git repositories, without an MCP server or a database.

Status: pre-release scaffold. The build spec is [`roadmap.md`](roadmap.md); example stores live in [`examples/`](examples/).

```bash
cargo run -p kipi-cli -- --help
```
