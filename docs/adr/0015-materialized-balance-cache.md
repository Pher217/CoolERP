# ADR-015 — Materialized balance cache, maintained by a trigger inside the posting transaction

- **Status:** Accepted (2026-06-13)

## Context
Summing balances on read is O(n) per account — the scale cliff that Formance,
Modern Treasury, and TigerBeetle all materialize past. It also could not express
per-currency balances after [ADR-014](0014-per-currency-double-entry.md).

## Decision
Account balances are read from a materialized `account_balances` cache keyed
`(account_id, currency)`, maintained by a trigger (`trg_bump_balance`,
`AFTER INSERT ON journal_lines`) that upserts incrementally **inside the posting
transaction** — not by the application, and not at read time.

`journal_lines` remains the **source of truth**; the cache is derived state.

## Consequences
- Balance lookup becomes an O(1) primary-key read.
- Because the cache is maintained in a trigger it **cannot be written out of
  band**, and it rolls back with the entry if the deferred balance check fails.
  Because `journal_lines` is append-only, an incremental `+=` cannot drift.
- Migration `0005_balances.sql` plus a backfill. New `GET /accounts/{code}/balances`.
- A reconciliation test asserts cache == fresh `SUM` (zero drift); an idempotency
  test asserts a replayed post does not double-count.
