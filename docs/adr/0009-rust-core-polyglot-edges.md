# ADR-009 — Rust at the core, polyglot at the edges

- **Status:** Accepted (2026-05-30)
- **Amends:** [ADR-003](0003-licensing-agpl-core-permissive-sdk-dco.md), [ADR-008](0008-human-ui-is-a-react-web-spa.md)

## Context
Forcing every contributor through the Rust compiler shrinks the contributor pool
the project depends on. But hot-path correctness genuinely demands Rust.
Precedents: AppFlowy (Rust core + Flutter UI), Zoo/KittyCAD (Rust/WASM core +
TypeScript app).

## Decision
CoolERP is **Rust-first, not Rust-only**.

- **Rust is mandatory for the core** — the invariant-enforcing layer:
  `ol-domain`, the posting engine (`ol-ledger`), the event store and replay
  (`ol-events`), and the database schema and invariants.
- **The edges are deliberately polyglot** — MCP speaks a language-agnostic
  protocol; client SDKs ship **Python and TypeScript first** (generated from the
  OpenAPI spec) with Rust secondary; the human UI is TypeScript/React.

The invariant is the **architecture** — text source-of-truth, invocable
capabilities, deterministic replay — not a language purity test.

## Consequences
- The **OpenAPI 3.1 spec becomes the linchpin**: one contract generates the TS UI
  client *and* the Python + TS SDKs.
- Python and TypeScript contributions are first-class at the SDK and tooling layer.
