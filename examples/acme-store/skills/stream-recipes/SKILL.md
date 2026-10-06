---
name: stream-recipes
description: "Use when adding or changing a Kafka consumer or producer in the events pipeline: idempotent sinks, retries, schema evolution."
x-kipi:
  scope: acme-ml
  tags: [streaming, kafka]
---

# stream-recipes

## Add a consumer

1. Add the Avro schema under `schemas/` with a versioned topic name.
2. Create `consumers/<topic>.rs` from the template; key the sink on `event_id`.
3. Configure retries with a dead-letter topic; never drop a message silently.

## Evolve a schema

Add fields with defaults only. A breaking change means a new topic version.
