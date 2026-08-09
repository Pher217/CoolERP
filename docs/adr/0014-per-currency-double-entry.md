# ADR-014 — Per-currency double-entry; reject cross-currency until FX exists

- **Status:** Accepted (2026-06-13)

## Context
A conformance review found a concrete money-loss hole: because the balance check
summed amounts **globally**, a USD-debit / EUR-credit entry with equal integer
amounts passed as "balanced". That silently destroys value.

## Decision
Journal lines carry an ISO-4217 `currency` (default `EUR`). The double-entry
invariant holds **within each currency** (Σdebit = Σcredit *per currency*),
enforced in both `ol_domain::assert_balanced` and the database triggers.

A genuine **cross-currency** entry — legs in different currencies that cannot
each balance — is **rejected, not recorded**. There is no FX rate yet to
reconcile the legs, so accepting it would destroy value. Refusing is the
conservative-correct choice.

## Consequences
- Migration `0004_currency.sql` adds `journal_lines.currency` and per-currency
  trigger bodies. `LedgerError::Unbalanced` names the offending currency.
- Per-line currency is plumbed through the API, MCP, and CLI.
- FX gain/loss and a functional-currency amount per line are deliberately
  deferred to a later change.
- Single-currency behaviour is unchanged (backwards compatible).
