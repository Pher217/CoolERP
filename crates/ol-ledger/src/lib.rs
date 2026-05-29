//! ol-ledger — posting engine, SQLx hot path, idempotency.
//!
//! [`post_journal_entry`] is the single write path into the ledger. It is the
//! credibility surface of OpenLedger, so correctness is enforced in layers:
//!
//! 1. **Pure pre-check** ([`ol_domain::assert_balanced`]) fails fast on
//!    unbalanced / malformed input before touching the database.
//! 2. **Idempotency by construction** — every post claims a row in
//!    `idempotency_keys (operation, idempotency_key)`. A duplicate key returns
//!    the stored result instead of creating a second entry; "ran twice ≠ paid
//!    twice". (architecture §1)
//! 3. **Isolation** — the posting transaction runs at REPEATABLE READ and takes
//!    `SELECT … FOR UPDATE` row locks on the touched account rows (ordered by id
//!    for deadlock-free locking). Serialization failures (SQLSTATE `40001`) and
//!    deadlocks (`40P01`) retry the whole transaction with bounded backoff.
//!    (ADR-005)
//! 4. **DB-enforced balance** — the deferred `CONSTRAINT TRIGGER` in
//!    `migrations/0001_init.sql` asserts `Σdebit = Σcredit` and `≥2 lines` at
//!    COMMIT. The Rust pre-check is defense in depth, never the only guard.
//! 5. **Audit log** — every successful post appends an append-only `events` row
//!    (actor, capability, inputs hash, resulting entry id). (architecture §1)
//!
//! All money is integer cents (`ol_domain::Cents`). No floats. (ADR-007)

use chrono::NaiveDate;
use ol_domain::{Cents, Line, assert_balanced};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

/// Operation name recorded in `idempotency_keys.operation` and `events.capability`.
const OPERATION: &str = "post_journal_entry";

/// Bounded retry budget for serialization failures / deadlocks / in-flight
/// idempotency races. (ADR-005)
const MAX_ATTEMPTS: u32 = 8;

/// A request to post one balanced journal entry.
///
/// `lines` use `ol_domain::Line` (account_code + debit/credit cents). The entry
/// is posted into the journal identified by `journal_code`.
#[derive(Debug, Clone)]
pub struct PostRequest {
    pub idempotency_key: Uuid,
    pub journal_code: String,
    pub entry_date: NaiveDate,
    pub memo: Option<String>,
    pub reference: Option<String>,
    /// Audit actor (e.g. the MCP token subject). Recorded in `events.actor`.
    pub actor: String,
    pub lines: Vec<Line>,
}

/// Result of a post. `replayed` is true when the call matched an existing
/// `idempotency_key` and returned the original entry instead of creating a new one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostResult {
    pub entry_id: i64,
    pub balanced: bool,
    #[serde(default)]
    pub replayed: bool,
}

/// Signed balance view of an account: raw `debits`/`credits` plus a
/// type-normalized `balance` (assets/expenses are debit-positive; liabilities,
/// equity and income are credit-positive).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Balance {
    pub account_code: String,
    pub debits: Cents,
    pub credits: Cents,
    pub balance: Cents,
}

/// Errors from the posting path. Codes mirror the MCP structured-error envelope
/// (see `api-surface.md`).
#[derive(Debug, thiserror::Error)]
pub enum PostError {
    /// Pre-check rejected the input (unbalanced, <2 lines, or a double-sided line).
    #[error(transparent)]
    Domain(#[from] ol_domain::LedgerError),
    /// A referenced account code does not exist.
    #[error("ACCOUNT_NOT_FOUND: {0}")]
    AccountNotFound(String),
    /// The target journal code does not exist.
    #[error("JOURNAL_NOT_FOUND: {0}")]
    JournalNotFound(String),
    /// The DB balance trigger rejected the entry at COMMIT (should be unreachable
    /// after the pre-check; surfaced if it ever fires).
    #[error("UNBALANCED_ENTRY: {message}")]
    Unbalanced { message: String },
    /// An append-only trigger rejected a mutation.
    #[error("APPEND_ONLY: {message}")]
    AppendOnly { message: String },
    /// Retries exhausted on serialization failure / deadlock / in-flight race.
    #[error("SERIALIZATION_FAILURE: exhausted {0} attempts")]
    Serialization(u32),
    /// Any other database error.
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// How a single attempt resolved, so the retry loop can decide to retry vs. return.
enum Outcome {
    Done(PostResult),
    /// Retryable: serialization failure, deadlock, or an in-flight duplicate whose
    /// result is not yet committed.
    Retry,
}

/// Post one balanced journal entry, idempotently.
///
/// Returns the created entry, or — if `idempotency_key` was already used for this
/// operation — the original entry with `replayed = true`. Unbalanced or malformed
/// input is rejected before any database work.
pub async fn post_journal_entry(pool: &PgPool, req: &PostRequest) -> Result<PostResult, PostError> {
    // Layer 1: pure pre-check. Fail fast, before opening a transaction.
    assert_balanced(&req.lines)?;

    for attempt in 0..MAX_ATTEMPTS {
        match try_post(pool, req).await {
            Ok(Outcome::Done(result)) => return Ok(result),
            Ok(Outcome::Retry) => {
                backoff(attempt).await;
                continue;
            }
            Err(PostError::Serialization(_)) => {
                backoff(attempt).await;
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    Err(PostError::Serialization(MAX_ATTEMPTS))
}

/// One transactional attempt. Any serialization-class failure is mapped to
/// [`Outcome::Retry`] (via [`PostError::Serialization`]) so the caller retries.
async fn try_post(pool: &PgPool, req: &PostRequest) -> Result<Outcome, PostError> {
    let mut tx = pool.begin().await.map_err(classify)?;

    // ADR-005: REPEATABLE READ for a stable snapshot; row locks below serialize
    // concurrent posts to the same accounts.
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
        .execute(&mut *tx)
        .await
        .map_err(classify)?;

    // Layer 2: claim the idempotency key. RETURNING 1 is Some(_) iff we inserted
    // the row (i.e. we own this post); None means the key already exists.
    let claimed: Option<i32> = sqlx::query_scalar(
        "INSERT INTO idempotency_keys (operation, idempotency_key) \
         VALUES ($1, $2) ON CONFLICT DO NOTHING RETURNING 1",
    )
    .bind(OPERATION)
    .bind(req.idempotency_key)
    .fetch_optional(&mut *tx)
    .await
    .map_err(classify)?;

    if claimed.is_none() {
        // The key exists. Read the stored result. If it is NULL the owning
        // transaction is still in flight (or its commit is not yet visible to our
        // snapshot) — roll back and retry; a fresh snapshot will see it.
        let stored: Option<serde_json::Value> = sqlx::query_scalar(
            "SELECT result FROM idempotency_keys WHERE operation = $1 AND idempotency_key = $2",
        )
        .bind(OPERATION)
        .bind(req.idempotency_key)
        .fetch_one(&mut *tx)
        .await
        .map_err(classify)?;

        return match stored {
            Some(json) => {
                tx.commit().await.map_err(classify)?;
                let mut result: PostResult = serde_json::from_value(json)
                    .map_err(|e| PostError::Db(sqlx::Error::Decode(Box::new(e))))?;
                result.replayed = true;
                Ok(Outcome::Done(result))
            }
            None => {
                let _ = tx.rollback().await;
                Ok(Outcome::Retry)
            }
        };
    }

    // We own this post. Resolve the journal.
    let journal_id: Option<i64> = sqlx::query_scalar("SELECT id FROM journals WHERE code = $1")
        .bind(&req.journal_code)
        .fetch_optional(&mut *tx)
        .await
        .map_err(classify)?;
    let journal_id =
        journal_id.ok_or_else(|| PostError::JournalNotFound(req.journal_code.clone()))?;

    // Layer 3: lock the touched account rows, ordered by id for deadlock-free
    // locking, and resolve code -> id in one pass.
    let mut codes: Vec<String> = req.lines.iter().map(|l| l.account_code.clone()).collect();
    codes.sort();
    codes.dedup();
    let rows: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, code FROM accounts WHERE code = ANY($1) ORDER BY id FOR UPDATE")
            .bind(&codes)
            .fetch_all(&mut *tx)
            .await
            .map_err(classify)?;

    if rows.len() != codes.len() {
        let found: std::collections::HashSet<&str> = rows.iter().map(|(_, c)| c.as_str()).collect();
        let missing = codes
            .iter()
            .find(|c| !found.contains(c.as_str()))
            .cloned()
            .unwrap_or_default();
        // Roll back the idempotency claim so a corrected retry can proceed.
        let _ = tx.rollback().await;
        return Err(PostError::AccountNotFound(missing));
    }
    let id_of: std::collections::HashMap<&str, i64> =
        rows.iter().map(|(id, c)| (c.as_str(), *id)).collect();

    // Insert the entry header.
    let entry_id: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (journal_id, entry_date, memo, reference) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(journal_id)
    .bind(req.entry_date)
    .bind(&req.memo)
    .bind(&req.reference)
    .fetch_one(&mut *tx)
    .await
    .map_err(classify)?;

    // Insert the lines. The deferred balance trigger checks them at COMMIT.
    for line in &req.lines {
        let account_id = id_of[line.account_code.as_str()];
        sqlx::query(
            "INSERT INTO journal_lines (entry_id, account_id, debit, credit) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(entry_id)
        .bind(account_id)
        .bind(line.debit)
        .bind(line.credit)
        .execute(&mut *tx)
        .await
        .map_err(classify)?;
    }

    // Layer 5: append the audit event.
    let result_ids = vec![entry_id];
    sqlx::query(
        "INSERT INTO events (actor, capability, inputs_hash, entity_id, result_ids) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&req.actor)
    .bind(OPERATION)
    .bind(inputs_hash(req))
    .bind(entry_id.to_string())
    .bind(&result_ids)
    .execute(&mut *tx)
    .await
    .map_err(classify)?;

    // Persist the result so a later replay returns the same entry.
    let result = PostResult {
        entry_id,
        balanced: true,
        replayed: false,
    };
    let result_json = serde_json::to_value(&result)
        .map_err(|e| PostError::Db(sqlx::Error::Encode(Box::new(e))))?;
    sqlx::query(
        "UPDATE idempotency_keys SET result = $3 \
         WHERE operation = $1 AND idempotency_key = $2",
    )
    .bind(OPERATION)
    .bind(req.idempotency_key)
    .bind(result_json)
    .execute(&mut *tx)
    .await
    .map_err(classify)?;

    // COMMIT — the deferred balance trigger fires here.
    tx.commit().await.map_err(classify)?;
    Ok(Outcome::Done(result))
}

/// Read an account's balance: raw debit/credit sums and the type-normalized
/// signed balance. Returns zeros for an account with no lines.
pub async fn account_balance(pool: &PgPool, account_code: &str) -> Result<Balance, PostError> {
    let row: Option<(String, i64, i64)> = sqlx::query_as(
        "SELECT a.type::text, \
                COALESCE(SUM(l.debit), 0)::bigint, \
                COALESCE(SUM(l.credit), 0)::bigint \
           FROM accounts a \
           LEFT JOIN journal_lines l ON l.account_id = a.id \
          WHERE a.code = $1 \
          GROUP BY a.id, a.type",
    )
    .bind(account_code)
    .fetch_optional(pool)
    .await?;

    let (acct_type, debits, credits) =
        row.ok_or_else(|| PostError::AccountNotFound(account_code.to_string()))?;

    // Assets and expenses are debit-positive; liabilities, equity, income are
    // credit-positive.
    let balance = match acct_type.as_str() {
        "asset" | "expense" => debits - credits,
        _ => credits - debits,
    };

    Ok(Balance {
        account_code: account_code.to_string(),
        debits,
        credits,
        balance,
    })
}

/// SHA-256 of the canonical request inputs, hex-encoded, for the audit trail.
fn inputs_hash(req: &PostRequest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(req.idempotency_key.as_bytes());
    hasher.update(req.journal_code.as_bytes());
    hasher.update(req.entry_date.to_string().as_bytes());
    hasher.update(req.memo.as_deref().unwrap_or("").as_bytes());
    hasher.update(req.reference.as_deref().unwrap_or("").as_bytes());
    for line in &req.lines {
        hasher.update(line.account_code.as_bytes());
        hasher.update(line.debit.to_le_bytes());
        hasher.update(line.credit.to_le_bytes());
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Map a serialization failure (`40001`) or deadlock (`40P01`) to a retryable
/// error, the balance/append-only triggers (`P0001`) to their typed variants,
/// and everything else to [`PostError::Db`].
fn classify(e: sqlx::Error) -> PostError {
    if let Some(db) = e.as_database_error() {
        let code = db.code().map(|c| c.into_owned());
        let message = db.message().to_string();
        match code.as_deref() {
            Some("40001") | Some("40P01") => return PostError::Serialization(0),
            Some("P0001") if message.contains("UNBALANCED_ENTRY") => {
                return PostError::Unbalanced { message };
            }
            Some("P0001") if message.contains("APPEND_ONLY") => {
                return PostError::AppendOnly { message };
            }
            _ => {}
        }
    }
    PostError::Db(e)
}

/// Exponential backoff with attempt-derived jitter (no wall-clock / RNG needed).
async fn backoff(attempt: u32) {
    let base_ms = 2_u64.saturating_pow(attempt).min(64);
    let jitter_ms = u64::from(attempt) * 3;
    tokio::time::sleep(std::time::Duration::from_millis(base_ms + jitter_ms)).await;
}
