# kipi

kipi is a single-binary CLI that gives LLM coding agents (Claude Code, Codex CLI, Cursor and any
agent that reads `SKILL.md` / `AGENTS.md`) a scoped map of all your local git repositories plus
your shared skills — without an MCP server, a daemon or a database.

It is for one person working across many repos, some personal and some for an employer, who wants
every agent session to know what the other repos are, where they live and which skills apply,
without pasting context or bloating every `CLAUDE.md`, and without work knowledge ever reaching a
personal repo (or the reverse).

**The bet.** An agent does not need a curated copy of another repo's knowledge; it needs a *map*.
A one-line registry entry — name, aliases, summary, local path — is enough for the agent to find
the real repo and read its actual code, README and specs, which are always current. kipi maintains
the map, links the skills and grants the agent read access to the repos it needs.

```text
## Known repos (acme)
- billing-api (billing, invoices) — Invoicing and payment orchestration; OpenAPI in api/openapi.yaml · ~/work/acme/billing-api
- events-pipeline (events, pipeline) — Kafka consumers that turn raw events into billing and ML features · ~/work/acme/ml/events-pipeline
```

Where to go next:

- [Install](install.md) kipi and its shell completions.
- [Quickstart](quickstart.md): from zero to ten synced repos.
- [Concepts](concepts.md): the five nouns, three cost tiers, four channels and grants.
- [Configuration](config-reference.md): every key of `bindings.toml`, `machine.toml` and
  `config.toml`.
- [FAQ](faq.md): why not cards, why not MCP.

The build spec is [`roadmap.md`](https://github.com/martin-conur/kipi/blob/main/roadmap.md); where
it leaves a choice open, the answer is in [Decisions](decisions.md).
