# AGENTS.md — CoolERP

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

## Hard rules
- **Money is integer cents (`i64` / `BIGINT`). Never floats.**
- **The ledger is append-only.** No `UPDATE`/`DELETE` on `journal_entries`, `journal_lines`, `events`. Corrections are reversing entries.
- **Every entry balances** (≥2 lines, Σdebit = Σcredit) — enforced by a DB trigger. Don't move this check into application code only.
- **Ledger posting, inventory receipt and starting a process instance are idempotent** via a client `idempotency_key` + a canonical request hash + `UNIQUE (operation, idempotency_key)`. `advance_process`'s key is server-derived and does not dedupe a retry — a retried advance is caught only incidentally by the illegal-transition guard.
- **No raw-SQL / `execute` MCP tool.** Capabilities are discrete and typed.
- **Posting path:** REPEATABLE READ + `SELECT … FOR UPDATE`, retry on SQLSTATE `40001`.
- Property-test the double-entry invariant hard before any release.

## Where things live
- Invariants: `crates/ol-domain/src/lib.rs` + `migrations/0001_init.sql`.
- Posting engine: `crates/ol-ledger` (SQLx). API: `crates/ol-api` (SQLx — ADR-004 anticipated SeaORM 1.1.x for CRUD, but no SeaORM is used today).
- MCP capabilities + OAuth model: `crates/ol-mcp`.
- ADRs live in [`docs/adr/`](docs/adr/README.md). Wider design notes live in the maintainer's vault, not in-repo.

## Conventions
- Commit prefixes: `feat: fix: refactor: docs: test: chore:`. No AI attribution in git history.
- AGPL-3.0 core; SDK is MIT OR Apache-2.0. New deps must pass `cargo deny check licenses`.
