//! ol-ledger — posting engine, SQLx hot path, idempotency.
//!
//! Stage 1 implementation plan (see vault `02 Projects/OpenERP`):
//! - `post_journal_entry(idempotency_key, ...)`: open a transaction at
//!   REPEATABLE READ, `SELECT ... FOR UPDATE` the touched account rows, insert
//!   entry + lines, let the deferred DB trigger assert balance at COMMIT, and
//!   retry the whole transaction on SQLSTATE 40001 (bounded). (ADR-005)
//! - Idempotency: `INSERT ... ON CONFLICT (operation, idempotency_key) DO NOTHING`
//!   then return the stored result on conflict — duplicate posts are impossible.
//! - All money is integer cents (`ol_domain::Cents`). (ADR-007)
