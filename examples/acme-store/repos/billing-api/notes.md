# billing-api notes

## Invariants

- Every mutating endpoint requires an `Idempotency-Key` header; keys live 24 h in the `idempotency_keys` table.
- Invoice totals are recomputed from line items on read; never persist a total by hand.

## Gotchas

- `/v1/charges` is deprecated and kept only for one legacy client; never call v1 from new code.

## Where to look

- `api/openapi.yaml`: the contract.
- `src/idempotency/`: key storage and replay.
