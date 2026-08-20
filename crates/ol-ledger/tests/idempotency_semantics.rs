//! Idempotency-key semantics — pins the contract for duplicate keys.
//!
//! These tests assert the EXACT outcome for every duplicate-key scenario:
//! exact `PostError` variant, exact row counts, exact entry ids. No ranges,
//! no substring checks.
//!
//! On the current code base two of the five cases still pass through as replays
//! (different actor, legacy NULL request_hash), so the file fails — that is
//! expected and documents the remaining gaps.

use ol_domain::Line;
use ol_ledger::{PostError, PostRequest, post_journal_entry};
use sqlx::PgPool;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const OPERATION: &str = "post_journal_entry";

fn line(code: &str, debit: i64, credit: i64) -> Line {
    Line::new(code, debit, credit)
}

fn req(key: Uuid, actor: &str, memo: Option<&str>, lines: Vec<Line>) -> PostRequest {
    PostRequest {
        idempotency_key: key,
        journal_code: "GEN".into(),
        entry_date: "2026-08-17".parse().unwrap(),
        effective_date: None,
        memo: memo.map(String::from),
        reference: None,
        actor: actor.into(),
        lines,
    }
}

async fn seed(pool: &PgPool) -> TestResult {
    sqlx::query(
        "INSERT INTO journals (code, name) VALUES ('GEN', 'General') ON CONFLICT (code) DO NOTHING",
    )
    .execute(pool)
    .await?;
    for (code, name, kind) in [("1000", "Cash", "asset"), ("4000", "Sales", "income")] {
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

async fn entry_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn idempotency_count(pool: &PgPool, key: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM idempotency_keys WHERE operation = $1 AND idempotency_key = $2",
    )
    .bind(OPERATION)
    .bind(key)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Add the `request_hash` column that the legacy-row test needs. The base
/// schema applied by #[sqlx::test] does not yet include it; the test creates
/// it locally so it can model a pre-migration row with a NULL hash.
async fn add_request_hash_column(pool: &PgPool) -> TestResult {
    sqlx::query("ALTER TABLE idempotency_keys ADD COLUMN IF NOT EXISTS request_hash TEXT")
        .execute(pool)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// (1) Same key with a materially different payload must be rejected.
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn reused_key_different_payload_is_rejected(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let key = Uuid::new_v4();
    let actor = "actor-a";

    let first = post_journal_entry(
        &pool,
        &req(
            key,
            actor,
            Some("first"),
            vec![line("1000", 5_000, 0), line("4000", 0, 5_000)],
        ),
    )
    .await?;
    assert!(!first.replayed);

    let second = post_journal_entry(
        &pool,
        &req(
            key,
            actor,
            Some("second"),
            vec![line("1000", 7_000, 0), line("4000", 0, 7_000)],
        ),
    )
    .await;

    assert!(
        second.is_err(),
        "a duplicate key with a different payload must be rejected"
    );
    let err = second.unwrap_err();
    assert!(
        matches!(err, PostError::IdempotencyKeyReused(_)),
        "expected exact IDEMPOTENCY_KEY_REUSED-class error, got {err:?}"
    );

    assert_eq!(
        entry_count(&pool).await,
        1,
        "only the first entry may exist"
    );
    assert_eq!(
        idempotency_count(&pool, key).await,
        1,
        "only one idempotency row"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// (2) Same key with an identical payload must still replay the stored result.
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn reused_key_identical_payload_replays(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let key = Uuid::new_v4();
    let actor = "actor-a";
    let lines = vec![line("1000", 3_000, 0), line("4000", 0, 3_000)];

    let first = post_journal_entry(&pool, &req(key, actor, Some("same"), lines.clone())).await?;
    assert!(!first.replayed);

    let second = post_journal_entry(&pool, &req(key, actor, Some("same"), lines)).await?;
    assert!(second.replayed, "identical payload must replay");
    assert_eq!(
        first.entry_id, second.entry_id,
        "replay must return the exact same entry id"
    );

    assert_eq!(entry_count(&pool).await, 1, "no second entry created");
    assert_eq!(idempotency_count(&pool, key).await, 1);
    Ok(())
}

// ---------------------------------------------------------------------------
// (3) Same key and payload but a different actor must be rejected.
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn reused_key_different_actor_is_rejected(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let key = Uuid::new_v4();
    let lines = vec![line("1000", 4_000, 0), line("4000", 0, 4_000)];

    let first = post_journal_entry(&pool, &req(key, "actor-a", Some("x"), lines.clone())).await?;
    assert!(!first.replayed);

    let second = post_journal_entry(&pool, &req(key, "actor-b", Some("x"), lines)).await;

    assert!(
        second.is_err(),
        "a duplicate key with a different actor must be rejected, not attributed to the first actor"
    );
    let err = second.unwrap_err();
    assert!(
        matches!(err, PostError::IdempotencyKeyReused(_)),
        "expected exact IDEMPOTENCY_KEY_REUSED-class error, got {err:?}"
    );

    assert_eq!(
        entry_count(&pool).await,
        1,
        "only the first entry may exist"
    );

    let event_actors: Vec<String> =
        sqlx::query_scalar("SELECT actor FROM events WHERE capability = $1 ORDER BY created_at")
            .bind(OPERATION)
            .fetch_all(&pool)
            .await?;
    assert_eq!(
        event_actors,
        vec!["actor-a"],
        "second actor must leave no audit trail"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// (4) memo = None and memo = Some("") are distinct payloads in Postgres.
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn reused_key_none_vs_empty_memo_are_different(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let key = Uuid::new_v4();
    let actor = "actor-a";
    let lines = vec![line("1000", 2_000, 0), line("4000", 0, 2_000)];

    let first = post_journal_entry(&pool, &req(key, actor, None, lines.clone())).await?;
    assert!(!first.replayed);

    let second = post_journal_entry(&pool, &req(key, actor, Some(""), lines)).await;

    assert!(
        second.is_err(),
        "Postgres stores NULL and empty string differently, so the payloads must differ"
    );
    let err = second.unwrap_err();
    assert!(
        matches!(err, PostError::IdempotencyKeyReused(_)),
        "expected exact IDEMPOTENCY_KEY_REUSED-class error, got {err:?}"
    );

    assert_eq!(
        entry_count(&pool).await,
        1,
        "only the first entry may exist"
    );
    assert_eq!(idempotency_count(&pool, key).await, 1);
    Ok(())
}

// ---------------------------------------------------------------------------
// (5) A legacy row whose request_hash is NULL fails closed.
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn legacy_null_request_hash_fails_closed(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    add_request_hash_column(&pool).await?;

    let key = Uuid::new_v4();
    let fake_result = serde_json::json!({
        "entry_id": 12345_i64,
        "balanced": true,
        "replayed": false,
    });

    sqlx::query(
        "INSERT INTO idempotency_keys (operation, idempotency_key, result, request_hash) \
         VALUES ($1, $2, $3, NULL)",
    )
    .bind(OPERATION)
    .bind(key)
    .bind(fake_result)
    .execute(&pool)
    .await?;

    let result = post_journal_entry(
        &pool,
        &req(
            key,
            "actor-z",
            Some("anything"),
            vec![line("1000", 1_000, 0), line("4000", 0, 1_000)],
        ),
    )
    .await;

    assert!(
        result.is_err(),
        "a legacy row with NULL request_hash must not accept an arbitrary payload as a replay"
    );
    let err = result.unwrap_err();
    assert!(
        matches!(err, PostError::IdempotencyKeyReused(_)),
        "expected exact IDEMPOTENCY_KEY_REUSED-class error, got {err:?}"
    );

    assert_eq!(
        entry_count(&pool).await,
        0,
        "no journal entry may be created from a legacy idempotency row"
    );
    assert_eq!(idempotency_count(&pool, key).await, 1);
    Ok(())
}
