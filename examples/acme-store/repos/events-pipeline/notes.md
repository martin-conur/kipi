# events-pipeline notes

## Invariants

- Consumers are at-least-once; every sink must be idempotent on `event_id`.

## Gotchas

- Topic names carry a schema version suffix (`.v2`); bump it, never mutate a schema in place.

## Where to look

- `consumers/`: one module per topic.
- `schemas/`: Avro schemas, the source of truth for event shapes.
