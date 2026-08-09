# CLAUDE.md — CoolERP (repo)

Repo-specific guidance. Project design docs, decisions, lessons, and research live in the maintainer's Obsidian vault under `02 Projects/CoolERP`, not in this repo.

## Commands
```bash
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all
sqlx migrate run                      # needs $DATABASE_URL
cargo sqlx prepare --check --workspace
cargo deny check
```
The `query!` macros are compile-time checked, so `cargo build` needs a live `$DATABASE_URL`
(there is no committed `.sqlx` offline cache). `make demo` brings up Postgres, migrates, and seeds.

Architecture decision records live in [`docs/adr/`](docs/adr/README.md) — the ADR-0XX citations
throughout the code resolve there.

## Non-negotiable invariants
- Money = integer cents (`BIGINT`/`i64`). No floats. (ADR-007)
- Append-only ledger + audit log; corrections are reversing entries. (DB triggers in `migrations/0001_init.sql`)
- Every entry balances (≥2 lines, Σdebit = Σcredit) — DB-enforced.
- Idempotency on every state change via `idempotency_key` + `UNIQUE (operation, idempotency_key)`.
- Posting: REPEATABLE READ + `SELECT … FOR UPDATE`, retry on `40001`. (ADR-005)
- ORM: SQLx hot path + SeaORM 1.1.x; do NOT pin SeaORM 2.0 (still RC). (ADR-004)
- MCP: no admin shell / raw-SQL tool; OAuth 2.1 PKCE; scoped tokens; 401+resource_metadata (MUST), 403 insufficient_scope (SHOULD); prefer Client ID Metadata Documents over RFC 7591. (ADR-006)

## Git
- Repo is **private** until public launch (ADR-002).
- Commit prefixes `feat:/fix:/refactor:/docs:/test:/chore:`. No AI attribution.
- Feature branch + PR for code changes; never commit secrets / `.env`.
