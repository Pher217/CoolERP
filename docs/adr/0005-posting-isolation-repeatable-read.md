# ADR-005 — Posting isolation: REPEATABLE READ + row locks, retry on 40001

- **Status:** Accepted (2026-05-29)

## Context
Concurrent posting must not corrupt account balances. Postgres SERIALIZABLE (SSI)
on a hot path generates false-positive aborts under contention, costing
throughput. SQLx exposes no first-class isolation API.

## Decision
Default the posting transaction to **REPEATABLE READ** plus
`SELECT … FOR UPDATE` on the touched account rows. Reserve SERIALIZABLE for
cross-account invariants that row locks cannot cover. Always catch SQLSTATE
**`40001`** (serialization failure) and retry the whole transaction, bounded.

## Consequences
- The posting engine wraps a bounded retry loop.
- Isolation is set via raw `SET TRANSACTION ISOLATION LEVEL …`.
- Serialization failures should be instrumented early; load-test before launch.
