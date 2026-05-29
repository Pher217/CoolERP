# AGENTS.md — OpenLedger

Guidance for AI coding agents contributing to this repo. (AAIF AGENTS.md standard.)

## What this is
An agent-native, double-entry-correct ERP. Rust workspace, PostgreSQL, MCP server. Accounting correctness is the entire credibility surface — a single double-entry bug is fatal to trust.

## Build / test
```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
# DB-backed checks need a Postgres at $DATABASE_URL:
sqlx migrate run
cargo sqlx prepare --check --workspace
cargo deny check
```
> A Rust toolchain was NOT installed when the scaffold was created. Install stable Rust first (`rustup`), then run a real `cargo build` and reconcile any dependency version/feature drift before relying on the manifests.

## Hard rules
- **Money is integer cents (`i64` / `BIGINT`). Never floats.**
- **The ledger is append-only.** No `UPDATE`/`DELETE` on `journal_entries`, `journal_lines`, `events`. Corrections are reversing entries.
- **Every entry balances** (≥2 lines, Σdebit = Σcredit) — enforced by a DB trigger. Don't move this check into application code only.
- **Every state-changing capability is idempotent** via a client `idempotency_key` + `UNIQUE (operation, idempotency_key)`.
- **No raw-SQL / `execute` MCP tool.** Capabilities are discrete and typed.
- **Posting path:** REPEATABLE READ + `SELECT … FOR UPDATE`, retry on SQLSTATE `40001`.
- Property-test the double-entry invariant hard before any release.

## Where things live
- Invariants: `crates/ol-domain/src/lib.rs` + `migrations/0001_init.sql`.
- Posting engine: `crates/ol-ledger` (SQLx). CRUD: `crates/ol-api` (SeaORM 1.1.x, NOT 2.0 — still RC).
- MCP capabilities + OAuth model: `crates/ol-mcp`.
- Capability surface reference + ADRs live in the maintainer's design notes (not in-repo).

## Conventions
- Commit prefixes: `feat: fix: refactor: docs: test: chore:`. No AI attribution in git history.
- AGPL-3.0 core; SDK is MIT OR Apache-2.0. New deps must pass `cargo deny check licenses`.
