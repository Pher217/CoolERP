//! Integration test proving inputs_hash is boundary-ambiguous today.
//!
//! The `inputs_hash` function in `src/lib.rs` concatenates variable-length fields
//! with no delimiter. Adjacent variable-length fields can produce identical byte
//! streams for distinct inputs, causing hash collisions.
//!
//! This test posts two requests that differ ONLY in where a field boundary falls
//! between adjacent variable-length fields in the hash:
//! 1. memo="AB", reference="C"  vs  memo="A", reference="BC"  (adjacent in hash)
//! 2. reference="AB", line1.account_code="C"  vs  reference="A", line1.account_code="BC"
//!    (adjacent in hash — reference and first line's account_code)
//!
//! It reads the persisted `events.inputs_hash` values and asserts they DIFFER.
//! This test MUST FAIL on unmodified code — the hashes are currently EQUAL, which
//! is the defect. A passing test here proves nothing.

use ol_domain::Line;
use ol_ledger::{PostRequest, post_journal_entry};
use sqlx::PgPool;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn seed(pool: &PgPool) -> TestResult {
    sqlx::query(
        "INSERT INTO journals (code, name) VALUES ('GEN', 'General') ON CONFLICT (code) DO NOTHING",
    )
    .execute(pool)
    .await?;
    for (code, name, kind) in [
        ("1000", "Cash", "asset"),
        ("C", "Account C", "asset"),
        ("BC", "Account BC", "asset"),
        ("4000", "Sales", "income"),
    ] {
        sqlx::query(
            "INSERT INTO accounts (code, name, type) VALUES ($1, $2, $3::account_type) ON CONFLICT (code) DO NOTHING",
        )
        .bind(code)
        .bind(name)
        .bind(kind)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Build a request with the given memo and reference, balanced with 1 line.
/// memo and reference are adjacent variable-length fields in inputs_hash.
fn req_mr(memo: &str, reference: &str, key: Uuid) -> PostRequest {
    PostRequest {
        idempotency_key: key,
        journal_code: "GEN".into(),
        entry_date: "2026-01-01".parse().unwrap(),
        effective_date: None,
        memo: Some(memo.into()),
        reference: Some(reference.into()),
        actor: "test-actor".into(),
        lines: vec![Line::new("1000", 1000, 0), Line::new("4000", 0, 1000)],
    }
}

/// Build a request with the given reference and first line's account_code.
/// reference and first line's account_code are adjacent variable-length fields in inputs_hash.
fn req_ra(reference: &str, account_code: &str, key: Uuid) -> PostRequest {
    PostRequest {
        idempotency_key: key,
        journal_code: "GEN".into(),
        entry_date: "2026-01-01".parse().unwrap(),
        effective_date: None,
        memo: Some("boundary-test".into()),
        reference: Some(reference.into()),
        actor: "test-actor".into(),
        lines: vec![
            Line {
                account_code: account_code.into(),
                debit: 1000,
                credit: 0,
                currency: "EUR".into(),
            },
            Line::new("4000", 0, 1000),
        ],
    }
}

/// Fetch the inputs_hash from the events table for the given entry_id.
async fn fetch_inputs_hash(pool: &PgPool, entry_id: i64) -> String {
    sqlx::query_scalar("SELECT inputs_hash FROM events WHERE entity_id = $1")
        .bind(entry_id.to_string())
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Post a request, capture inputs_hash, then delete ONLY the idempotency_keys row
/// so the same key can be reused for a different payload.
/// The events table is append-only (trigger) — we leave both event rows in place.
async fn post_and_capture_hash(pool: &PgPool, req: &PostRequest) -> String {
    let result = post_journal_entry(pool, req).await.unwrap();
    let hash = fetch_inputs_hash(pool, result.entry_id).await;

    // Delete ONLY the idempotency_keys row to allow key reuse.
    // journal_entries/journal_lines/events are append-only and stay.
    sqlx::query("DELETE FROM idempotency_keys WHERE operation = $1 AND idempotency_key = $2")
        .bind("post_journal_entry")
        .bind(req.idempotency_key)
        .execute(pool)
        .await
        .unwrap();

    hash
}

#[sqlx::test(migrations = "../../migrations")]
async fn inputs_hash_boundary_ambiguity_memo_reference(pool: PgPool) -> TestResult {
    seed(&pool).await?;

    // Use a FIXED idempotency_key for both requests so the ONLY difference
    // is the memo/reference boundary (adjacent variable-length fields in hash).
    let fixed_key = Uuid::nil();

    // GIVEN two requests differing only in where the boundary falls between
    // memo and reference: ("AB", "C") vs ("A", "BC")
    let hash1 = post_and_capture_hash(&pool, &req_mr("AB", "C", fixed_key)).await;
    let hash2 = post_and_capture_hash(&pool, &req_mr("A", "BC", fixed_key)).await;

    // THEN the hashes must differ (they are distinct inputs)
    // This assertion FAILS today because inputs_hash concatenates without a delimiter.
    assert_ne!(
        hash1, hash2,
        "inputs_hash collision: (memo=AB, reference=C) vs (memo=A, reference=BC) produced identical hashes"
    );
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn inputs_hash_boundary_ambiguity_reference_account_code(pool: PgPool) -> TestResult {
    seed(&pool).await?;

    // Use a FIXED idempotency_key for both requests so the ONLY difference
    // is the reference/account_code boundary (adjacent variable-length fields in hash).
    let fixed_key = Uuid::nil();

    // GIVEN two requests differing only in where the boundary falls between
    // reference and first line's account_code: ("AB", "C") vs ("A", "BC")
    let hash1 = post_and_capture_hash(&pool, &req_ra("AB", "C", fixed_key)).await;
    let hash2 = post_and_capture_hash(&pool, &req_ra("A", "BC", fixed_key)).await;

    // THEN the hashes must differ (they are distinct inputs)
    // This assertion FAILS today because inputs_hash concatenates without a delimiter.
    assert_ne!(
        hash1, hash2,
        "inputs_hash collision: (reference=AB, account_code=C) vs (reference=A, account_code=BC) produced identical hashes"
    );
    Ok(())
}
