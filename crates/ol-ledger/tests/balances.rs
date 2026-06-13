//! Balance-cache integration tests: reconciliation, idempotency, and
//! per-currency listing via `account_balance` / `account_balances_all`.
//!
//! Every test gets a fresh, migrated database (migrations 0001–0005).
//! Pattern follows tests/posting.rs and tests/currency.rs.

use ol_domain::Line;
use ol_ledger::{
    PostError, PostRequest, account_balance, account_balances_all, post_journal_entry,
};
use sqlx::PgPool;
use uuid::Uuid;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn req(key: Uuid, lines: Vec<Line>) -> PostRequest {
    PostRequest {
        idempotency_key: key,
        journal_code: "GEN".into(),
        entry_date: "2026-06-13".parse().unwrap(),
        effective_date: None,
        memo: Some("balance test".into()),
        reference: None,
        actor: "test-actor".into(),
        lines,
    }
}

/// Seed journal + EUR and USD accounts.
/// Accounts default to currency = 'EUR' per migration 0001.
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
            "INSERT INTO accounts (code, name, type) VALUES ($1, $2, $3::account_type) \
             ON CONFLICT (code) DO NOTHING",
        )
        .bind(code)
        .bind(name)
        .bind(kind)
        .execute(pool)
        .await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Test (a): reconciliation — cache == fresh SUM per (account_id, currency)
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn cache_matches_sum_on_read(pool: PgPool) -> TestResult {
    // GIVEN several posted entries: EUR-only entries and a 4-line multi-currency
    //       entry (EUR pair + USD pair)
    // WHEN we query account_balances cache rows
    // THEN for every (account_id, currency) row the cache equals
    //      a fresh SELECT SUM(debit), SUM(credit) FROM journal_lines — zero drift
    seed(&pool).await?;

    // EUR-only entries
    post_journal_entry(
        &pool,
        &req(
            Uuid::new_v4(),
            vec![
                Line::in_currency("1000", 10_000, 0, "EUR"),
                Line::in_currency("4000", 0, 10_000, "EUR"),
            ],
        ),
    )
    .await?;

    post_journal_entry(
        &pool,
        &req(
            Uuid::new_v4(),
            vec![
                Line::in_currency("1000", 5_000, 0, "EUR"),
                Line::in_currency("4000", 0, 5_000, "EUR"),
            ],
        ),
    )
    .await?;

    // 4-line multi-currency entry: EUR pair + USD pair
    post_journal_entry(
        &pool,
        &req(
            Uuid::new_v4(),
            vec![
                Line::in_currency("1000", 3_000, 0, "EUR"),
                Line::in_currency("4000", 0, 3_000, "EUR"),
                Line::in_currency("1001", 2_000, 0, "USD"),
                Line::in_currency("4001", 0, 2_000, "USD"),
            ],
        ),
    )
    .await?;

    // Read the cache rows
    let cache_rows: Vec<(i64, String, i64, i64)> = sqlx::query_as(
        "SELECT account_id, currency, debits, credits FROM account_balances ORDER BY account_id, currency",
    )
    .fetch_all(&pool)
    .await?;

    // Compute fresh SUM per (account_id, currency) from the source of truth
    let sum_rows: Vec<(i64, String, i64, i64)> = sqlx::query_as(
        "SELECT account_id, currency, \
                SUM(debit)::bigint, SUM(credit)::bigint \
           FROM journal_lines \
          GROUP BY account_id, currency \
          ORDER BY account_id, currency",
    )
    .fetch_all(&pool)
    .await?;

    assert_eq!(
        cache_rows.len(),
        sum_rows.len(),
        "cache and SUM must have the same number of rows"
    );

    for (cache, sum) in cache_rows.iter().zip(sum_rows.iter()) {
        assert_eq!(
            cache, sum,
            "cache drift detected for (account_id={}, currency={}): cache={:?}, sum={:?}",
            cache.0, cache.1, cache, sum
        );
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Test (b): idempotent replay does NOT double-count the cache
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn idempotent_replay_does_not_double_count_cache(pool: PgPool) -> TestResult {
    // GIVEN an entry posted with a specific idempotency key
    // WHEN the same idempotency key is submitted again (replayed=true)
    // THEN the account_balances cache reflects exactly ONE entry, not two
    seed(&pool).await?;

    let key = Uuid::new_v4();
    let lines = vec![
        Line::in_currency("1000", 7_500, 0, "EUR"),
        Line::in_currency("4000", 0, 7_500, "EUR"),
    ];

    let first = post_journal_entry(&pool, &req(key, lines.clone())).await?;
    assert!(!first.replayed, "first post must not be a replay");

    let second = post_journal_entry(&pool, &req(key, lines)).await?;
    assert!(second.replayed, "second post must be flagged as replay");
    assert_eq!(
        first.entry_id, second.entry_id,
        "replayed entry_id must match"
    );

    // Cache must reflect exactly one entry's worth of debits/credits
    let cash_debits: i64 = sqlx::query_scalar(
        "SELECT debits FROM account_balances \
          WHERE account_id = (SELECT id FROM accounts WHERE code = '1000') \
            AND currency = 'EUR'",
    )
    .fetch_one(&pool)
    .await?;

    assert_eq!(
        cash_debits, 7_500,
        "cache must reflect exactly one post (7500), not a double-count (15000)"
    );

    let sales_credits: i64 = sqlx::query_scalar(
        "SELECT credits FROM account_balances \
          WHERE account_id = (SELECT id FROM accounts WHERE code = '4000') \
            AND currency = 'EUR'",
    )
    .fetch_one(&pool)
    .await?;

    assert_eq!(
        sales_credits, 7_500,
        "cache must reflect exactly one post (7500), not a double-count (15000)"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Test (c): account_balance returns correct O(1) value with currency field
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn account_balance_returns_o1_value_with_currency(pool: PgPool) -> TestResult {
    // GIVEN an EUR entry posted to cash (1000) and sales (4000)
    // WHEN account_balance is called for '1000'
    // THEN it returns debits=10_000, credits=0, balance=10_000, currency="EUR"
    seed(&pool).await?;

    post_journal_entry(
        &pool,
        &req(
            Uuid::new_v4(),
            vec![
                Line::in_currency("1000", 10_000, 0, "EUR"),
                Line::in_currency("4000", 0, 10_000, "EUR"),
            ],
        ),
    )
    .await?;

    let bal = account_balance(&pool, "1000").await?;

    assert_eq!(bal.account_code, "1000");
    assert_eq!(bal.currency, "EUR");
    assert_eq!(bal.debits, 10_000);
    assert_eq!(bal.credits, 0);
    assert_eq!(bal.balance, 10_000, "asset: debit-positive");

    Ok(())
}

// ---------------------------------------------------------------------------
// Test (d): account_balances_all — multi-currency and unknown account
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn account_balances_all_returns_two_rows_for_multi_currency_account(
    pool: PgPool,
) -> TestResult {
    // GIVEN an account ('1000') posted in both EUR and USD currencies
    //       (two separate entries)
    // WHEN account_balances_all is called for '1000'
    // THEN two rows are returned, ordered by currency (EUR before USD),
    //      each with correct debits/credits/balance
    seed(&pool).await?;

    // EUR entry into account 1000
    post_journal_entry(
        &pool,
        &req(
            Uuid::new_v4(),
            vec![
                Line::in_currency("1000", 8_000, 0, "EUR"),
                Line::in_currency("4000", 0, 8_000, "EUR"),
            ],
        ),
    )
    .await?;

    // USD entry into account 1000
    post_journal_entry(
        &pool,
        &req(
            Uuid::new_v4(),
            vec![
                Line::in_currency("1000", 3_000, 0, "USD"),
                Line::in_currency("4001", 0, 3_000, "USD"),
            ],
        ),
    )
    .await?;

    let bals = account_balances_all(&pool, "1000").await?;

    assert_eq!(bals.len(), 2, "must have exactly two currency rows");

    // Ordered by currency: EUR < USD alphabetically
    assert_eq!(bals[0].currency, "EUR");
    assert_eq!(bals[0].debits, 8_000);
    assert_eq!(bals[0].credits, 0);
    assert_eq!(bals[0].balance, 8_000); // asset: debit-positive

    assert_eq!(bals[1].currency, "USD");
    assert_eq!(bals[1].debits, 3_000);
    assert_eq!(bals[1].credits, 0);
    assert_eq!(bals[1].balance, 3_000); // asset: debit-positive

    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn account_balances_all_unknown_account_returns_not_found(pool: PgPool) -> TestResult {
    // GIVEN no accounts seeded for code '9999'
    // WHEN account_balances_all is called
    // THEN PostError::AccountNotFound is returned
    seed(&pool).await?;

    let err = account_balances_all(&pool, "9999").await.unwrap_err();

    assert!(
        matches!(err, PostError::AccountNotFound(ref c) if c == "9999"),
        "expected AccountNotFound(9999), got {err:?}"
    );

    Ok(())
}
