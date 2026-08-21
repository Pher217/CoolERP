//! Integration test pinning that period enforcement FAILS CLOSED.
//!
//! A single test asserts three exact outcomes so the contract cannot regress
//! piecemeal:
//!   1. effective_date OUTSIDE every configured period is REFUSED with a
//!      structured error and NO entry row is created;
//!   2. effective_date INSIDE an open period still SUCCEEDS;
//!   3. effective_date INSIDE a closed period is still REFUSED.
//!
//! Case (1) is expected to FAIL today: the DB trigger `assert_period_open`
//! (migrations/0003_periods.sql) only blocks effective_dates that fall in a
//! *closed* period, so a date matching no period currently posts freely. That
//! failure is correct and pins the desired fail-closed behaviour until the
//! posting path enforces it.
//!
//! Fixtures and style mirror `tests/periods.rs`.

use ol_domain::Line;
use ol_ledger::{
    PostError, PostRequest,
    periods::{close_period, create_period},
    post_journal_entry,
};
use sqlx::PgPool;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn line(code: &str, debit: i64, credit: i64) -> Line {
    Line::new(code, debit, credit)
}

fn req_with_effective(key: Uuid, effective: Option<chrono::NaiveDate>) -> PostRequest {
    PostRequest {
        idempotency_key: key,
        journal_code: "GEN".into(),
        entry_date: "2026-06-15".parse().unwrap(),
        effective_date: effective,
        memo: Some("period fail-closed test".into()),
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

#[sqlx::test(migrations = "../../migrations")]
async fn period_enforcement_fails_closed(pool: PgPool) -> TestResult {
    // GIVEN one open fiscal period covering 2026-06-01..2026-06-30 and nothing
    // covering, e.g., 2026-08-15.
    seed(&pool).await?;
    create_period(
        &pool,
        "2026-06",
        "2026-06-01".parse().unwrap(),
        "2026-06-30".parse().unwrap(),
    )
    .await?;
    assert_eq!(entry_count(&pool).await, 0);

    // -----------------------------------------------------------------
    // Case 2: effective_date INSIDE an open period -> Ok, one entry.
    // -----------------------------------------------------------------
    let result = post_journal_entry(
        &pool,
        &req_with_effective(Uuid::new_v4(), Some("2026-06-15".parse().unwrap())),
    )
    .await?;

    assert!(result.balanced, "open-period post must succeed");
    assert_eq!(
        entry_count(&pool).await,
        1,
        "exactly one entry after open post"
    );

    // -----------------------------------------------------------------
    // Case 3: effective_date INSIDE a closed period -> PeriodClosed, no entry.
    // -----------------------------------------------------------------
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
    assert_eq!(
        entry_count(&pool).await,
        1,
        "closed-period post must not create an entry"
    );

    // -----------------------------------------------------------------
    // Case 1: effective_date OUTSIDE every configured period (2026-08-15)
    //         -> REFUSED with a structured error, NO entry created.
    //
    // This is expected to FAIL today: 2026-08-15 matches no fiscal period, so
    // the close-period trigger does not fire and the entry posts freely. The
    // assertion pins the fail-closed contract the posting path must enforce.
    // -----------------------------------------------------------------
    let outside = "2026-08-15".parse::<chrono::NaiveDate>().unwrap();
    let err = post_journal_entry(&pool, &req_with_effective(Uuid::new_v4(), Some(outside)))
        .await
        .unwrap_err();

    assert!(
        matches!(err, PostError::PeriodNotFound(ref d) if *d == outside.to_string()),
        "expected PeriodNotFound(\"{outside}\"), got {err:?}"
    );
    assert_eq!(
        entry_count(&pool).await,
        1,
        "out-of-period post must not create an entry"
    );
    Ok(())
}
