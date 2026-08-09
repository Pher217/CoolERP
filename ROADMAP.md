# Roadmap

Public roadmap for CoolERP. Tracked in more detail on the GitHub Projects board
(to be created) and via [`good first issue`](https://github.com/Pher217/CoolERP/labels/good%20first%20issue) labels.

> Status: **private, pre-launch.** The repo goes public at Stage 3.

## Stage 0 — Scaffold (weeks 0–2) ✅
- [x] Cargo monorepo scaffold (`ol-domain/ledger/events/api/mcp/sdk/cli`)
- [x] PostgreSQL schema + double-entry invariants enforced in-DB (deferred balance
      constraint trigger + append-only triggers)
- [x] `ol-domain` invariant + property test; `cargo build`/`test`/`fmt`/`clippy` green
- [x] License + supply-chain policy (`deny.toml`), DCO, AGENTS.md
- [ ] CI workflow live (pending `workflow` OAuth scope)

## Stage 1 — Ledger + MCP (weeks 2–8)
- [ ] Posting engine: `post_journal_entry` (REPEATABLE READ + `SELECT … FOR UPDATE`,
      retry on `40001`), idempotency via `UNIQUE (operation, idempotency_key)`
- [ ] Capabilities: accounts, balances, journal listing, AR/AP, payments, stock moves
- [ ] MCP server (`rmcp`): typed capabilities, OAuth 2.1 PKCE, scoped tokens
- [ ] `ol-sdk`: typed request/response + structured error envelope
- [ ] Smoke tests green: post→balance, replay determinism, MCP scope enforcement, idempotency

## Stage 2 — UI + replay (weeks 8–12)
- [ ] `ol-events` projections + deterministic replay
- [ ] Tauri desktop UI (read-mostly ledger / invoice / audit views)
- [ ] Manifesto README polish; recruit 3 named maintainers

## Stage 3 — Launch
- [ ] Public release (repo goes public), simultaneous HN + r/rust + X
- [ ] Public roadmap board, `good first issue` triage, list on PulseMCP / mcp.directory

## Explicitly out of v0.1
Manufacturing/MRP, payroll, HR, CRM, projects, multi-entity consolidation, VAT/GST &
e-invoicing regimes (Peppol/XRechnung/SAF-T), IFRS/GAAP mapping, multi-currency.

## Re-scope triggers
- If v0.1 needs VAT / e-invoicing before traction → vertical/regional pivot; re-scope
  before building compliance.
- If a well-funded incumbent open-sources an architecturally identical AI-native ERP →
  reassess before doubling down.
