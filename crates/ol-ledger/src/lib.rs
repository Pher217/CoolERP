//! ol-ledger — posting engine, SQLx hot path, idempotency.
//!
//! [`post_journal_entry`] is the single write path into the ledger. It is the
//! credibility surface of CoolERP, so correctness is enforced in layers:
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

pub mod periods;

use chrono::NaiveDate;
use ol_domain::{Cents, Line, assert_balanced};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

/// Operation name recorded in `idempotency_keys.operation` and `events.capability`.
const OPERATION: &str = "post_journal_entry";

/// Bounded retry budget for serialization failures / deadlocks / in-flight
/// idempotency races. (ADR-005) Heavy-contention tuning + load test is a
/// pre-launch task.
const MAX_ATTEMPTS: u32 = 10;

/// A request to post one balanced journal entry.
///
/// `lines` use `ol_domain::Line` (account_code + debit/credit cents). The entry
/// is posted into the journal identified by `journal_code`.
#[derive(Debug, Clone)]
pub struct PostRequest {
    pub idempotency_key: Uuid,
    pub journal_code: String,
    pub entry_date: NaiveDate,
    /// Economic date determining the fiscal period. Defaults to entry_date when None.
    pub effective_date: Option<NaiveDate>,
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
    pub currency: String,
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
    /// effective_date falls in a closed fiscal period.
    #[error("PERIOD_CLOSED: {message}")]
    PeriodClosed { message: String },
    /// A fiscal period with overlapping dates already exists.
    #[error("PERIOD_OVERLAP: {message}")]
    PeriodOverlap { message: String },
    /// The named fiscal period does not exist.
    #[error("PERIOD_NOT_FOUND: {0}")]
    PeriodNotFound(String),
    /// The idempotency key was already used for a different payload.
    #[error("IDEMPOTENCY_KEY_REUSED: {0}")]
    IdempotencyKeyReused(String),
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
    let request_hash = request_hash(req);

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
        "INSERT INTO idempotency_keys (operation, idempotency_key, request_hash) \
         VALUES ($1, $2, $3) ON CONFLICT DO NOTHING RETURNING 1",
    )
    .bind(OPERATION)
    .bind(req.idempotency_key)
    .bind(&request_hash)
    .fetch_optional(&mut *tx)
    .await
    .map_err(classify)?;

    if claimed.is_none() {
        // The key already exists — this is a replay. We must NOT read the stored
        // result inside this transaction: our REPEATABLE READ snapshot was fixed
        // at the INSERT above, and the owning transaction may have committed
        // *after* that snapshot. `ON CONFLICT DO NOTHING` resolves the conflict
        // against a fresh visibility check (hence it returned no row), but a
        // SELECT here would run against our older snapshot and see the row as
        // invisible — yielding a spurious "not found" on a perfectly valid
        // retry. Instead, roll back and read with a fresh snapshot on the pool
        // (READ COMMITTED). Because the claim row and its result are written in
        // the *same* transaction (below), a visible row always has a non-NULL
        // result; absence of a visible result means the owner is still in flight,
        // so we retry with backoff.
        let _ = tx.rollback().await;
        let stored: Option<(Option<String>, Option<serde_json::Value>)> = sqlx::query_as(
            "SELECT request_hash, result FROM idempotency_keys WHERE operation = $1 AND idempotency_key = $2",
        )
        .bind(OPERATION)
        .bind(req.idempotency_key)
        .fetch_optional(pool)
        .await
        .map_err(classify)?;

        return match stored {
            // Same payload as the stored request: replay the stored result.
            Some((Some(stored_hash), Some(json))) if stored_hash == request_hash => {
                let mut result: PostResult = serde_json::from_value(json)
                    .map_err(|e| PostError::Db(sqlx::Error::Decode(Box::new(e))))?;
                result.replayed = true;
                Ok(Outcome::Done(result))
            }
            // Same payload, but the owner has not committed its result yet — retry.
            Some((Some(stored_hash), None)) if stored_hash == request_hash => Ok(Outcome::Retry),
            // Different payload, or a legacy row with no recorded hash: fail closed.
            Some((Some(_), _)) | Some((None, _)) => Err(PostError::IdempotencyKeyReused(format!(
                "idempotency key {} already used with a different payload",
                req.idempotency_key
            ))),
            // Owner still in flight (row not yet committed/visible) — retry.
            None => Ok(Outcome::Retry),
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

    // Fail closed: when fiscal periods are configured, an effective_date that
    // matches none of them is refused, exactly as a closed-period post is. The
    // DB trigger (migrations/0003_periods.sql) only blocks dates inside a
    // *closed* period, so a date that slips into a gap between configured
    // periods would otherwise post freely. Posting stays opt-in while no
    // period is configured at all (see migrations/0003_periods.sql).
    let effective = req.effective_date.unwrap_or(req.entry_date);
    let (has_periods, covered): (bool, bool) = sqlx::query_as(
        "SELECT EXISTS(SELECT 1 FROM fiscal_periods), \
                EXISTS(SELECT 1 FROM fiscal_periods WHERE $1 BETWEEN start_date AND end_date)",
    )
    .bind(effective)
    .fetch_one(&mut *tx)
    .await
    .map_err(classify)?;

    if has_periods && !covered {
        let _ = tx.rollback().await;
        return Err(PostError::PeriodNotFound(effective.to_string()));
    }

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
        "INSERT INTO journal_entries (journal_id, entry_date, effective_date, memo, reference) \
         VALUES ($1, $2, COALESCE($3, $2), $4, $5) RETURNING id",
    )
    .bind(journal_id)
    .bind(req.entry_date)
    .bind(req.effective_date)
    .bind(&req.memo)
    .bind(&req.reference)
    .fetch_one(&mut *tx)
    .await
    .map_err(classify)?;

    // Insert the lines. The deferred balance trigger checks them at COMMIT.
    for line in &req.lines {
        let account_id = id_of[line.account_code.as_str()];
        sqlx::query(
            "INSERT INTO journal_lines (entry_id, account_id, debit, credit, currency) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(entry_id)
        .bind(account_id)
        .bind(line.debit)
        .bind(line.credit)
        .bind(&line.currency)
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

/// Read an account's balance from the O(1) materialized cache for the
/// account's native currency (accounts.currency).
///
/// Returns zero debits/credits when the account exists but has no posted lines.
pub async fn account_balance(pool: &PgPool, account_code: &str) -> Result<Balance, PostError> {
    let row: Option<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT a.type::text, a.currency, \
                COALESCE(b.debits,  0)::bigint, \
                COALESCE(b.credits, 0)::bigint \
           FROM accounts a \
           LEFT JOIN account_balances b \
                  ON b.account_id = a.id AND b.currency = a.currency \
          WHERE a.code = $1",
    )
    .bind(account_code)
    .fetch_optional(pool)
    .await?;

    let (acct_type, currency, debits, credits) =
        row.ok_or_else(|| PostError::AccountNotFound(account_code.to_string()))?;

    // Assets and expenses are debit-positive; liabilities, equity, income are
    // credit-positive.
    let balance = match acct_type.as_str() {
        "asset" | "expense" => debits - credits,
        _ => credits - debits,
    };

    Ok(Balance {
        account_code: account_code.to_string(),
        currency,
        debits,
        credits,
        balance,
    })
}

/// Return all per-currency balances for an account from the materialized cache,
/// ordered by currency. Returns an empty Vec when the account exists but has no
/// posted lines. Returns `PostError::AccountNotFound` when the code is unknown.
pub async fn account_balances_all(
    pool: &PgPool,
    account_code: &str,
) -> Result<Vec<Balance>, PostError> {
    // Check existence first so we can distinguish "no lines" from "unknown account".
    let exists: Option<i32> = sqlx::query_scalar("SELECT 1 FROM accounts WHERE code = $1")
        .bind(account_code)
        .fetch_optional(pool)
        .await?;
    if exists.is_none() {
        return Err(PostError::AccountNotFound(account_code.to_string()));
    }

    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT a.type::text, b.currency, b.debits::bigint, b.credits::bigint \
           FROM accounts a \
           JOIN account_balances b ON b.account_id = a.id \
          WHERE a.code = $1 \
          ORDER BY b.currency",
    )
    .bind(account_code)
    .fetch_all(pool)
    .await?;

    let balances = rows
        .into_iter()
        .map(|(acct_type, currency, debits, credits)| {
            let balance = match acct_type.as_str() {
                "asset" | "expense" => debits - credits,
                _ => credits - debits,
            };
            Balance {
                account_code: account_code.to_string(),
                currency,
                debits,
                credits,
                balance,
            }
        })
        .collect();

    Ok(balances)
}

/// SHA-256 of the canonical request inputs, hex-encoded, for the audit trail.
fn inputs_hash(req: &PostRequest) -> String {
    let mut hasher = Sha256::new();
    hasher.update(req.idempotency_key.as_bytes());
    hasher.update(req.journal_code.as_bytes());
    hasher.update(req.entry_date.to_string().as_bytes());
    if let Some(d) = req.effective_date {
        hasher.update(d.to_string().as_bytes());
    }
    hasher.update(req.memo.as_deref().unwrap_or("").as_bytes());
    hasher.update(req.reference.as_deref().unwrap_or("").as_bytes());
    for line in &req.lines {
        hasher.update(line.account_code.as_bytes());
        hasher.update(line.debit.to_le_bytes());
        hasher.update(line.credit.to_le_bytes());
        hasher.update(line.currency.as_bytes());
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Canonical SHA-256 request hash for idempotency replay.
///
/// * Includes `actor` so a key reused under a different actor is rejected.
/// * Does NOT collapse `None` and `Some("")` — they hash differently, matching
///   the way Postgres stores `NULL` and `''` distinctly.
/// * Sorts lines into a canonical order so reordering the same economic lines
///   does not produce a different hash.
/// * Versioned (`v: "1"`) so a future schema change can bump the format and
///   fail closed on old stored hashes.
fn request_hash(req: &PostRequest) -> String {
    #[derive(Serialize)]
    struct CanonicalLine {
        code: String,
        debit: i64,
        credit: i64,
        currency: String,
    }

    #[derive(Serialize)]
    struct CanonicalRequest<'a> {
        v: &'static str,
        op: &'static str,
        key: String,
        actor: &'a str,
        journal: &'a str,
        date: String,
        eff: Option<String>,
        memo: Option<String>,
        reference: Option<String>,
        lines: Vec<CanonicalLine>,
    }

    let mut lines: Vec<CanonicalLine> = req
        .lines
        .iter()
        .map(|l| CanonicalLine {
            code: l.account_code.clone(),
            debit: l.debit,
            credit: l.credit,
            currency: l.currency.clone(),
        })
        .collect();
    lines.sort_by(|a, b| {
        (&a.code, a.debit, a.credit, &a.currency).cmp(&(&b.code, b.debit, b.credit, &b.currency))
    });

    let canonical = CanonicalRequest {
        v: "1",
        op: OPERATION,
        key: req.idempotency_key.to_string(),
        actor: &req.actor,
        journal: &req.journal_code,
        date: req.entry_date.to_string(),
        eff: req.effective_date.map(|d| d.to_string()),
        memo: req.memo.clone(),
        reference: req.reference.clone(),
        lines,
    };

    let bytes = serde_json::to_vec(&canonical).expect("canonical request serializes");
    sha256_hex(&bytes)
}

/// Stable SHA-256 hex digest of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("{digest:x}")
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
            Some("P0001") if message.contains("PERIOD_CLOSED") => {
                return PostError::PeriodClosed { message };
            }
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
    let base_ms = 2_u64.saturating_pow(attempt).min(256);
    let jitter_ms = u64::from(attempt) * 3;
    tokio::time::sleep(std::time::Duration::from_millis(base_ms + jitter_ms)).await;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// GIVEN a fixed PostRequest whose every hashed field is populated,
    /// WHEN inputs_hash() is computed,
    /// THEN it equals a pinned SHA-256 hex vector.
    ///
    /// `inputs_hash` is persisted to the append-only `events` table, so its
    /// output is part of the audit trail forever. Any change to the hashed
    /// field set, their order, or the digest stack would silently break
    /// continuity with rows already written. SHA-256 itself is fixed by
    /// FIPS 180-4, so this vector must survive any sha2/digest version bump --
    /// if this test fails after a dependency update, the update changed
    /// behaviour and must not be merged.
    ///
    /// The vector was verified against an independent SHA-256 implementation
    /// over the same documented field order, so it pins the algorithm rather
    /// than merely echoing whatever this code currently emits.
    #[test]
    fn inputs_hash_matches_pinned_vector() {
        let req = PostRequest {
            idempotency_key: Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap(),
            journal_code: "GEN".to_string(),
            entry_date: NaiveDate::from_ymd_opt(2026, 1, 31).unwrap(),
            effective_date: Some(NaiveDate::from_ymd_opt(2026, 2, 1).unwrap()),
            memo: Some("pinned vector".to_string()),
            reference: Some("REF-1".to_string()),
            actor: "test".to_string(),
            lines: vec![Line::new("1000", 250_000, 0), Line::new("4000", 0, 250_000)],
        };

        assert_eq!(
            inputs_hash(&req),
            "b845e02b6d47d8bfe7d4b499c0cdbf2c3a58672677aa1cb86432676722389361",
            "inputs_hash changed -- the persisted audit digest is not stable"
        );
    }

    /// GIVEN two requests differing only in where a boundary falls between two
    /// adjacent variable-length string fields,
    /// WHEN both are hashed,
    /// THEN they must not collide.
    ///
    /// The hash concatenates variable-length fields with no delimiter, so
    /// (journal_code="AB", memo="C") and (journal_code="A", memo="BC") feed the
    /// same byte stream. This test documents that weakness; it is expected to
    /// FAIL until a delimiter is introduced. See the linked issue.
    #[test]
    #[ignore = "known defect: undelimited concatenation allows boundary collisions; tracked in #53"]
    fn inputs_hash_is_not_boundary_ambiguous() {
        let base = |journal: &str, memo: &str| PostRequest {
            idempotency_key: Uuid::nil(),
            journal_code: journal.to_string(),
            entry_date: NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
            effective_date: None,
            memo: Some(memo.to_string()),
            reference: None,
            actor: "test".to_string(),
            lines: vec![],
        };

        assert_ne!(
            inputs_hash(&base("AB", "C")),
            inputs_hash(&base("A", "BC")),
            "distinct inputs produced the same audit digest"
        );
    }
}
