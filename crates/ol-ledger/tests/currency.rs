//! Multi-currency integration tests for the posting engine.
//!
//! Each `#[sqlx::test]` gets a fresh, migrated database (migrations 0001–0004).
//! Pattern follows `tests/posting.rs`.

use ol_domain::{LedgerError, Line};
use ol_ledger::{PostError, PostRequest, post_journal_entry};
use sqlx::PgPool;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn req(key: Uuid, lines: Vec<Line>) -> PostRequest {
    PostRequest {
        idempotency_key: key,
        journal_code: "GEN".into(),
        entry_date: "2026-06-13".parse().unwrap(),
        effective_date: None,
        memo: Some("currency test".into()),
        reference: None,
        actor: "test-actor".into(),
        lines,
    }
}

/// Seed journal + EUR and USD accounts.
async fn seed(pool: &PgPool) -> TestResult {
    sqlx::query(
        "INSERT INTO journals (code, name) VALUES ('GEN', 'General') ON CONFLICT (code) DO NOTHING",
    )
    .execute(pool)
    .await?;
    for (code, name, kind) in [
        ("1000", "Cash EUR", "asset"),
        ("4000", "Sales EUR", "income"),
        ("1001", "Cash USD", "asset"),
        ("4001", "Sales USD", "income"),
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

async fn entry_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries")
        .fetch_one(pool)
        .await
        .unwrap()
}

// ---------------------------------------------------------------------------
// Test a: single-currency default EUR entry still succeeds (back-compat)
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn single_currency_eur_default_succeeds(pool: PgPool) -> TestResult {
    // GIVEN a two-line EUR entry using the default currency
    // WHEN posted
    // THEN it succeeds and one entry is written
    seed(&pool).await?;

    let result = post_journal_entry(
        &pool,
        &req(
            Uuid::new_v4(),
            vec![Line::new("1000", 10_000, 0), Line::new("4000", 0, 10_000)],
        ),
    )
    .await?;

    assert!(result.balanced);
    assert!(!result.replayed);
    assert_eq!(entry_count(&pool).await, 1);
    Ok(())
}

// ---------------------------------------------------------------------------
// Test b: multi-currency entry with both EUR and USD balanced -> Ok
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn multi_currency_each_balanced_succeeds(pool: PgPool) -> TestResult {
    // GIVEN a four-line entry: EUR pair (1000/4000) + USD pair (1001/4001),
    //       each currency balancing on its own
    // WHEN posted
    // THEN it succeeds and one entry is written
    seed(&pool).await?;

    let lines = vec![
        Line::in_currency("1000", 10_000, 0, "EUR"),
        Line::in_currency("4000", 0, 10_000, "EUR"),
        Line::in_currency("1001", 5_000, 0, "USD"),
        Line::in_currency("4001", 0, 5_000, "USD"),
    ];

    let result = post_journal_entry(&pool, &req(Uuid::new_v4(), lines)).await?;

    assert!(result.balanced);
    assert_eq!(entry_count(&pool).await, 1);
    Ok(())
}

// ---------------------------------------------------------------------------
// Test c: cross-currency entry (USD debit vs EUR credit) -> Err before DB
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn cross_currency_entry_rejected_by_precheck(pool: PgPool) -> TestResult {
    // GIVEN a two-line entry: 100 USD debit and 100 EUR credit (globally equal
    //       integer values, but neither currency balances on its own — no FX rate)
    // WHEN posted
    // THEN the Rust pre-check rejects it as PostError::Domain(LedgerError::Unbalanced)
    //      and no entry is written to the DB
    seed(&pool).await?;

    let lines = vec![
        Line::in_currency("1001", 10_000, 0, "USD"),
        Line::in_currency("4000", 0, 10_000, "EUR"),
    ];

    let err = post_journal_entry(&pool, &req(Uuid::new_v4(), lines))
        .await
        .unwrap_err();

    assert!(
        matches!(err, PostError::Domain(LedgerError::Unbalanced { .. })),
        "expected Domain(Unbalanced), got {err:?}"
    );
    assert_eq!(entry_count(&pool).await, 0, "no entry must be written");
    Ok(())
}

// ---------------------------------------------------------------------------
// Test d: DB trigger backstop — raw cross-currency INSERT fails at COMMIT
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn db_trigger_rejects_cross_currency_lines(pool: PgPool) -> TestResult {
    // GIVEN a journal entry header + two journal_lines that are cross-currency
    //       (USD debit and EUR credit, bypassing the Rust pre-check via raw SQL)
    // WHEN the transaction is committed
    // THEN the DB trigger fires and the COMMIT fails with "UNBALANCED_ENTRY"
    seed(&pool).await?;

    let jid: i64 = sqlx::query_scalar("SELECT id FROM journals WHERE code = 'GEN'")
        .fetch_one(&pool)
        .await?;
    let usd_cash: i64 = sqlx::query_scalar("SELECT id FROM accounts WHERE code = '1001'")
        .fetch_one(&pool)
        .await?;
    let eur_sales: i64 = sqlx::query_scalar("SELECT id FROM accounts WHERE code = '4000'")
        .fetch_one(&pool)
        .await?;

    let mut tx = pool.begin().await?;
    let eid: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (journal_id, entry_date, effective_date) \
         VALUES ($1, '2026-06-13', '2026-06-13') RETURNING id",
    )
    .bind(jid)
    .fetch_one(&mut *tx)
    .await?;

    // USD debit
    sqlx::query(
        "INSERT INTO journal_lines (entry_id, account_id, debit, credit, currency) \
         VALUES ($1, $2, 10000, 0, 'USD')",
    )
    .bind(eid)
    .bind(usd_cash)
    .execute(&mut *tx)
    .await?;

    // EUR credit — different currency, so per-currency sums never balance
    sqlx::query(
        "INSERT INTO journal_lines (entry_id, account_id, debit, credit, currency) \
         VALUES ($1, $2, 0, 10000, 'EUR')",
    )
    .bind(eid)
    .bind(eur_sales)
    .execute(&mut *tx)
    .await?;

    let commit_result = tx.commit().await;
    assert!(
        commit_result.is_err(),
        "cross-currency entry must be rejected at COMMIT by the DB trigger"
    );

    let db_err = commit_result.unwrap_err();
    let msg = db_err.to_string();
    assert!(
        msg.contains("UNBALANCED_ENTRY"),
        "DB error message must contain UNBALANCED_ENTRY, got: {msg}"
    );

    assert_eq!(entry_count(&pool).await, 0, "no entry must be persisted");
    Ok(())
}
