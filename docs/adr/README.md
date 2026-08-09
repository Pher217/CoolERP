# CoolERP Architecture Decision Records

This directory contains the [Nygard-format](https://cognitect.com/blog/2011/11/15/documenting-architecture-decisions) architecture decision records for CoolERP. The numbering has gaps because only decisions still cited by the code, plus recent significant reversals and naming decisions, have been ported into the repo. The full decision history lives in the maintainer's private notes.

| ADR | Title | Status | Summary |
|---|---|---|---|
| [ADR-002](0002-repo-private-until-launch.md) | Repo private until launch | Accepted (2026-05-29) · superseded in practice once the repo goes public | Keep the repository private until launch; flipping visibility is an explicit operator decision. |
| [ADR-003](0003-licensing-agpl-core-permissive-sdk-dco.md) | Licensing: AGPL-3.0 core, MIT/Apache SDK, DCO from day one | Accepted (2026-05-29) | Core is AGPL-3.0, SDK is MIT OR Apache-2.0, and external contributions require a DCO sign-off. |
| [ADR-004](0004-orm-sqlx-hot-path-seaorm-1x.md) | ORM: SQLx on the hot path, SeaORM 1.1.x for CRUD (not SeaORM 2.0) | Accepted (2026-05-29) | SQLx for the ledger hot path, SeaORM pinned to 1.1.x for CRUD; avoid the still-RC 2.0 and the duplicate sqlx 0.9 graph. |
| [ADR-005](0005-posting-isolation-repeatable-read.md) | Posting isolation: REPEATABLE READ + row locks, retry on 40001 | Accepted (2026-05-29) | Default posting transactions to REPEATABLE READ with row locks, reserve SERIALIZABLE when required, and retry SQLSTATE 40001. |
| [ADR-006](0006-mcp-auth-cimd-over-rfc7591.md) | MCP auth: Client ID Metadata Documents over RFC 7591; 403 is SHOULD | Accepted (2026-05-29) · **NOT YET IMPLEMENTED** | Designed MCP auth with Client ID Metadata Documents, RFC 7591 fallback, and required metadata endpoints; the API currently runs unauthenticated. |
| [ADR-007](0007-money-integer-cents.md) | Money is integer cents (BIGINT) | Accepted (2026-05-29) — **load-bearing invariant** | All money is integer cents as BIGINT/i64; no FLOAT, no NUMERIC unless sub-cent precision is genuinely needed. |
| [ADR-008](0008-human-ui-is-a-react-web-spa.md) | The human view is a web SPA (React 19 + Vite + TypeScript) | Accepted (2026-05-29) · supersedes the earlier Tauri-desktop choice | Human UI is a React 19 + Vite + TypeScript web SPA; Mermaid diagrams are generated from workflow YAML and UI auth is gated on ADR-006. |
| [ADR-009](0009-rust-core-polyglot-edges.md) | Rust at the core, polyglot at the edges | Accepted (2026-05-30) | Rust is mandatory for the invariant core; Python and TypeScript SDKs, MCP, and the React UI live at the edges. |
| [ADR-014](0014-per-currency-double-entry.md) | Per-currency double-entry; reject cross-currency until FX exists | Accepted (2026-06-13) | Double-entry invariant holds per currency; cross-currency entries are rejected until FX support exists. |
| [ADR-015](0015-materialized-balance-cache.md) | Materialized balance cache, maintained by a trigger inside the posting transaction | Accepted (2026-06-13) | `account_balances` is materialized by a trigger inside the posting transaction; `journal_lines` remains the source of truth. |
| [ADR-021](0021-least-privilege-db-role.md) | Least-privilege application database role (`ol_app`) | Accepted (2026-07-16) | Application connects as a non-owner role that can never UPDATE/DELETE/TRUNCATE append-only ledger tables. |
| [ADR-022](0022-honest-security-posture.md) | Honest security posture pre-launch | Accepted (2026-07-16) | Default API binds localhost, README claims are corrected to match reality, and DCO wording is fixed to clarify no relicensing rights. |
| [ADR-023](0023-public-name-is-coolerp.md) | Public name is "CoolERP" | Accepted (2026-08-05) · **supersedes ADR-010**, closing the ADR-001 → ADR-010 thread | Public product and brand name is CoolERP; crate names remain `ol-*` and the GitHub repository URL is unchanged for now. |

**ADR-006 is designed but not implemented.** The API and MCP server currently run unauthenticated in development mode. See [SECURITY.md](../../SECURITY.md) for the current posture.
