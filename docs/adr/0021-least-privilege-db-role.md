# ADR-021 — Least-privilege application database role (`ol_app`)

- **Status:** Accepted (2026-07-16)

## Context
The append-only `forbid_mutation()` triggers are necessary but **not sufficient**:
`TRUNCATE` does not fire row-level `BEFORE UPDATE/DELETE` triggers, and a table
**owner** can `ALTER TABLE … DISABLE TRIGGER`. Both holes bypass the ledger's
append-only guarantee entirely.

## Decision
The application connects as a **non-owner** role, `ol_app`:

- `SELECT` / `INSERT` on all tables.
- `UPDATE` only on the genuinely mutable state tables (`idempotency_keys`,
  `process_instances`, `fiscal_periods`, `account_balances`).
- **Never** `UPDATE` / `DELETE` / `TRUNCATE` on the append-only tables
  (`journal_entries`, `journal_lines`, `events`, `process_steps_log`).

Migrations continue to run as the schema owner, so DDL is unaffected.

## Consequences
- A privilege-layer boundary closes both holes **independent of trigger state**.
- Migration `0008_app_role.sql`; adversarial proof script
  `scripts/verify_append_only.sh` (11 checks: a legitimate post succeeds, every
  ledger mutation is denied, state-table updates are allowed).
- Deployments create a LOGIN user and `GRANT ol_app TO <user>`.
