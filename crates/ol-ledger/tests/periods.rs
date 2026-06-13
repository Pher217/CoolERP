//! Integration tests for fiscal period management and the period-close posting gate.
//!
//! Each test gets a fresh, fully-migrated ephemeral database (migrations 0001–0003).
//! Pattern follows `tests/posting.rs`.

use ol_domain::Line;
use ol_ledger::{
    PostError, PostRequest,
    periods::{close_period, create_period, reopen_period},
    post_journal_entry,
};
use sqlx::PgPool;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn line(code: &str, debit: i64, credit: i64) -> Line {
    Line {
        account_code: code.into(),
        debit,
        credit,
    }
}

fn req_with_effective(key: Uuid, effective: Option<chrono::NaiveDate>) -> PostRequest {
    PostRequest {
        idempotency_key: key,
        journal_code: "GEN".into(),
        entry_date: "2026-06-15".parse().unwrap(),
        effective_date: effective,
        memo: Some("periods test".into()),
        reference: None,
        actor: "test-actor".into(),
        lines: vec![line("1000", 10_000, 0), line("4000", 0, 10_000)],
    }
}

/// Seed a general journal and minimal chart of accounts.
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

// ---------------------------------------------------------------------------
// Test 1: post with effective_date inside an OPEN period -> Ok
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn post_into_open_period_succeeds(pool: PgPool) -> TestResult {
    // GIVEN an open fiscal period covering 2026-06-01..2026-06-30
    // WHEN posting an entry with effective_date inside that period
    // THEN the entry is created successfully
    seed(&pool).await?;
    create_period(
        &pool,
        "2026-06",
        "2026-06-01".parse().unwrap(),
        "2026-06-30".parse().unwrap(),
    )
    .await?;

    let result = post_journal_entry(
        &pool,
        &req_with_effective(Uuid::new_v4(), Some("2026-06-15".parse().unwrap())),
    )
    .await?;

    assert!(result.balanced);
    assert_eq!(entry_count(&pool).await, 1);
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 2: post with effective_date inside a CLOSED period -> PeriodClosed, no entry
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn post_into_closed_period_rejected(pool: PgPool) -> TestResult {
    // GIVEN a closed fiscal period covering 2026-06-01..2026-06-30
    // WHEN posting an entry with effective_date inside that closed period
    // THEN PostError::PeriodClosed is returned and no entry is written
    seed(&pool).await?;
    create_period(
        &pool,
        "2026-06",
        "2026-06-01".parse().unwrap(),
        "2026-06-30".parse().unwrap(),
    )
    .await?;
    close_period(&pool, "2026-06").await?;

    let err = post_journal_entry(
        &pool,
        &req_with_effective(Uuid::new_v4(), Some("2026-06-15".parse().unwrap())),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, PostError::PeriodClosed { .. }),
        "expected PeriodClosed, got {err:?}"
    );
    assert_eq!(entry_count(&pool).await, 0, "no entry must be written");
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 3: post with effective_date in a date covered by NO period -> Ok
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn post_with_no_covering_period_succeeds(pool: PgPool) -> TestResult {
    // GIVEN no fiscal period exists for 2026-06-15
    // WHEN posting an entry with effective_date on that date
    // THEN the entry is created successfully (periods are opt-in)
    seed(&pool).await?;

    let result = post_journal_entry(
        &pool,
        &req_with_effective(Uuid::new_v4(), Some("2026-06-15".parse().unwrap())),
    )
    .await?;

    assert!(result.balanced);
    assert_eq!(entry_count(&pool).await, 1);
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 4: effective_date: None -> stored effective_date == entry_date
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn effective_date_defaults_to_entry_date(pool: PgPool) -> TestResult {
    // GIVEN a PostRequest with effective_date: None and entry_date 2026-06-15
    // WHEN the entry is posted
    // THEN journal_entries.effective_date equals entry_date (2026-06-15)
    seed(&pool).await?;

    let result = post_journal_entry(&pool, &req_with_effective(Uuid::new_v4(), None)).await?;

    let stored_effective: chrono::NaiveDate =
        sqlx::query_scalar("SELECT effective_date FROM journal_entries WHERE id = $1")
            .bind(result.entry_id)
            .fetch_one(&pool)
            .await?;

    assert_eq!(
        stored_effective,
        "2026-06-15".parse::<chrono::NaiveDate>().unwrap(),
        "effective_date must default to entry_date"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 5: close then reopen: fail while closed, succeed after reopen
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn close_then_reopen_allows_posting(pool: PgPool) -> TestResult {
    // GIVEN a period that is closed and then reopened
    // WHEN posting fails while closed AND succeeds after reopen
    seed(&pool).await?;
    create_period(
        &pool,
        "2026-06",
        "2026-06-01".parse().unwrap(),
        "2026-06-30".parse().unwrap(),
    )
    .await?;
    close_period(&pool, "2026-06").await?;

    // While closed: must fail
    let err = post_journal_entry(
        &pool,
        &req_with_effective(Uuid::new_v4(), Some("2026-06-10".parse().unwrap())),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, PostError::PeriodClosed { .. }),
        "expected PeriodClosed while closed, got {err:?}"
    );

    // Reopen the period
    reopen_period(&pool, "2026-06").await?;

    // After reopen: must succeed
    let result = post_journal_entry(
        &pool,
        &req_with_effective(Uuid::new_v4(), Some("2026-06-10".parse().unwrap())),
    )
    .await?;
    assert!(result.balanced);
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 6: create two overlapping periods -> second is PeriodOverlap
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn overlapping_period_rejected(pool: PgPool) -> TestResult {
    // GIVEN a period 2026-06-01..2026-06-30 already exists
    // WHEN creating a second period 2026-06-15..2026-07-15 that overlaps it
    // THEN PostError::PeriodOverlap is returned
    create_period(
        &pool,
        "2026-06",
        "2026-06-01".parse().unwrap(),
        "2026-06-30".parse().unwrap(),
    )
    .await?;

    let err = create_period(
        &pool,
        "2026-06b",
        "2026-06-15".parse().unwrap(),
        "2026-07-15".parse().unwrap(),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, PostError::PeriodOverlap { .. }),
        "expected PeriodOverlap, got {err:?}"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 7: close_period on non-existent code -> PeriodNotFound
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn close_nonexistent_period_returns_not_found(pool: PgPool) -> TestResult {
    // GIVEN no fiscal period with code "NOPE" exists
    // WHEN close_period("NOPE") is called
    // THEN PostError::PeriodNotFound is returned
    let err = close_period(&pool, "NOPE").await.unwrap_err();

    assert!(
        matches!(err, PostError::PeriodNotFound(ref c) if c == "NOPE"),
        "expected PeriodNotFound(\"NOPE\"), got {err:?}"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 8: post on the START boundary of a closed period -> PeriodClosed, no entry
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn post_on_closed_period_start_boundary_rejected(pool: PgPool) -> TestResult {
    // GIVEN a closed fiscal period covering 2026-06-01..2026-06-30
    // WHEN posting an entry with effective_date exactly on the start boundary
    // THEN PostError::PeriodClosed is returned and no entry is written
    seed(&pool).await?;
    create_period(
        &pool,
        "2026-06",
        "2026-06-01".parse().unwrap(),
        "2026-06-30".parse().unwrap(),
    )
    .await?;
    close_period(&pool, "2026-06").await?;

    let err = post_journal_entry(
        &pool,
        &req_with_effective(Uuid::new_v4(), Some("2026-06-01".parse().unwrap())),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, PostError::PeriodClosed { .. }),
        "expected PeriodClosed on start boundary, got {err:?}"
    );
    assert_eq!(entry_count(&pool).await, 0, "no entry must be written");
    Ok(())
}

// ---------------------------------------------------------------------------
// Test 9: post on the END boundary of a closed period -> PeriodClosed, no entry
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn post_on_closed_period_end_boundary_rejected(pool: PgPool) -> TestResult {
    // GIVEN a closed fiscal period covering 2026-06-01..2026-06-30
    // WHEN posting an entry with effective_date exactly on the end boundary
    // THEN PostError::PeriodClosed is returned and no entry is written
    seed(&pool).await?;
    create_period(
        &pool,
        "2026-06",
        "2026-06-01".parse().unwrap(),
        "2026-06-30".parse().unwrap(),
    )
    .await?;
    close_period(&pool, "2026-06").await?;

    let err = post_journal_entry(
        &pool,
        &req_with_effective(Uuid::new_v4(), Some("2026-06-30".parse().unwrap())),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, PostError::PeriodClosed { .. }),
        "expected PeriodClosed on end boundary, got {err:?}"
    );
    assert_eq!(entry_count(&pool).await, 0, "no entry must be written");
    Ok(())
}
