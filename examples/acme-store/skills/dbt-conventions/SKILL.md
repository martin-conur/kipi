---
name: dbt-conventions
description: Use when writing or reviewing dbt models, tests or sources in this organization.
x-kipi:
  scope: acme
  tags: [data, sql]
---

# dbt-conventions

- Staging models are `stg_<source>__<entity>`, one per source table, no joins.
- Every model has a `unique` and `not_null` test on its primary key.
- Marts never select from sources directly; go through staging.

See `references/naming.md` for the full naming table.
