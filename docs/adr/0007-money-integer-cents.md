# ADR-007 — Money is integer cents (BIGINT)

- **Status:** Accepted (2026-05-29) — **load-bearing invariant**

## Context
Floating-point money loses value. This is the single most common correctness
defect in accounting software, and it is unrecoverable once written to a ledger.

## Decision
All monetary values are **integer cents**, stored as `BIGINT` / handled as `i64`.
Currency code is stored separately (see [ADR-014](0014-per-currency-double-entry.md)).
**Never** `MONEY`, **never** `FLOAT`/`NUMERIC` for amounts. `NUMERIC` is reserved
only if sub-cent precision is ever genuinely required.

## Consequences
- Every amount crossing the API, MCP, CLI, and UI boundary is integer cents.
- Formatting happens at the display edge only (`Intl.NumberFormat` in the UI) —
  integer cents in, formatted string out, no float ever constructed.
