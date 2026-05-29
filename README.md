# OpenLedger

> **OpenLedger is an ERP whose primary operator is an AI agent: double-entry-correct by construction, every state change a typed, idempotent capability, every figure traceable to an append-only event log. A duplicate journal entry is not a bug to catch — it is impossible to create.**

Rust core · PostgreSQL · Linux-first · MCP server day one · event-sourced · audit-log-first.

> **Status:** private, pre-launch. Stage 0 scaffold (2026-05-29). Not yet built or published. The repo is `openerp` as a working name; the product/public name is **OpenLedger**.

## The architectural inversion

In a traditional ERP, the human clicks through a UI and the database trusts the UI to keep the books straight. OpenLedger inverts this: the **AI agent is the primary operator**, driving the ledger through a typed [MCP](https://modelcontextprotocol.io) capability surface, while correctness is enforced **in the database**, not the UI.

- **Double-entry enforced in PostgreSQL.** Every journal entry has ≥2 lines and `sum(debits) = sum(credits)`, checked by a deferred constraint trigger. Entries are append-only — corrections are reversing entries, never edits.
- **The journal is the event log.** Balances and reports are CQRS projections, droppable and deterministically rebuildable by replay.
- **Idempotent by construction.** Every state-changing capability takes a client `idempotency_key`; a `UNIQUE (operation, idempotency_key)` constraint makes a duplicate post physically impossible. Ran twice ≠ paid twice.
- **Audit-log-first.** An append-only `events` table records actor, capability, inputs hash, and resulting entry IDs — machine-legible and replayable.

## vs. SAP / Odoo
Double-entry correctness lives in the database, not the UI. A duplicate journal entry is impossible to create. The full ledger is audit-replayable. (Kin, not foils: [TigerBeetle](https://github.com/tigerbeetle/tigerbeetle), [Formance Ledger](https://github.com/formancehq/ledger), Modern Treasury, Beancount — the ledger-as-event-log thesis is well-trodden; our edge is the agent-native capability surface + idempotency-by-construction.)

## Scope (v0.1)
**Ships:** chart of accounts, journals & general ledger, AR/AP (customers, vendors, bills, payments), basic inventory (items, stock moves, valuation), basic invoicing, audit trails.
**Out:** manufacturing/MRP, payroll, HR, CRM, projects, multi-entity consolidation, VAT/e-invoicing regimes, multi-currency.

## Workspace layout
```
crates/
  ol-domain/   double-entry invariants (AGPL-3.0)
  ol-ledger/   posting engine, SQLx hot path, idempotency (AGPL-3.0)
  ol-events/   event store + projections/replay (AGPL-3.0)
  ol-api/      Axum REST + SeaORM (AGPL-3.0)
  ol-mcp/      MCP server, OAuth 2.1 PKCE, scoped tokens (AGPL-3.0)
  ol-sdk/      typed client SDK (MIT OR Apache-2.0)
  ol-cli/      `ol serve|migrate|replay|post` (AGPL-3.0)
apps/ol-ui/    Tauri desktop UI — Stage 2 (AGPL-3.0)
migrations/    SQL schema + double-entry triggers
processes/     business-process source-of-truth (YAML)
```

## Stack
Axum 0.8 / Tokio · SQLx 0.9 (hot ledger path) + SeaORM 1.1.x (CRUD) · `rmcp` 1.7 MCP server · PostgreSQL · Tauri v2 (desktop UI). See the maintainer's design notes for the full rationale and ADRs.

## Licensing
Core/server/API/MCP/UI/CLI: **AGPL-3.0**. SDK: **MIT OR Apache-2.0**. Contributions require a [DCO](CONTRIBUTING.md) sign-off.

## Roadmap
- **Stage 0** (wk 0–2): monorepo scaffold, Postgres schema + double-entry trigger, CI with `cargo deny`. ← *here*
- **Stage 1** (wk 2–8): MCP server + capability list; posting engine + idempotency; smoke tests green.
- **Stage 2** (wk 8–12): Tauri UI; event replay; manifesto; recruit maintainers.
- **Stage 3**: public launch.
