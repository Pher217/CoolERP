//! Postgres-backed integration tests for the posting engine.
//!
//! Each `#[sqlx::test]` gets a fresh, migrated database (schema + DB triggers
//! from `migrations/0001_init.sql`). Requires `DATABASE_URL` pointing at a
//! Postgres server with CREATEDB rights; CI provides one, locally use the
//! dedicated test container (see PR / session note).

use futures::future::join_all;
use ol_domain::Line;
use ol_ledger::{PostError, PostRequest, account_balance, post_journal_entry};
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

fn req(key: Uuid, lines: Vec<Line>) -> PostRequest {
    PostRequest {
        idempotency_key: key,
        journal_code: "GEN".into(),
        entry_date: "2026-05-29".parse().unwrap(),
        memo: Some("test".into()),
        reference: None,
        actor: "test-actor".into(),
        lines,
    }
}

/// Seed a general journal and a minimal chart of accounts.
/// Uses ON CONFLICT DO NOTHING so this coexists with 0002_seed_chart_of_accounts.sql
/// which #[sqlx::test] applies (via migrations = "../../migrations") before each test.
async fn seed(pool: &PgPool) -> TestResult {
    sqlx::query(
        "INSERT INTO journals (code, name) VALUES ('GEN', 'General') ON CONFLICT (code) DO NOTHING",
    )
    .execute(pool)
    .await?;
    for (code, name, kind) in [
        ("1000", "Cash", "asset"),
        ("4000", "Sales", "income"),
        ("2000", "Accounts Payable", "liability"),
        ("5000", "COGS", "expense"),
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

async fn account_id(pool: &PgPool, code: &str) -> i64 {
    sqlx::query_scalar("SELECT id FROM accounts WHERE code = $1")
        .bind(code)
        .fetch_one(pool)
        .await
        .unwrap()
}

// --- Smoke test #1: post → query balance → assert --------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn post_then_balance(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let r = post_journal_entry(
        &pool,
        &req(
            Uuid::new_v4(),
            vec![line("1000", 10_000, 0), line("4000", 0, 10_000)],
        ),
    )
    .await?;

    assert!(r.balanced);
    assert!(!r.replayed);
    assert_eq!(entry_count(&pool).await, 1);

    let cash = account_balance(&pool, "1000").await?;
    assert_eq!(cash.debits, 10_000);
    assert_eq!(cash.credits, 0);
    assert_eq!(cash.balance, 10_000); // asset: debit-positive

    let sales = account_balance(&pool, "4000").await?;
    assert_eq!(sales.balance, 10_000); // income: credit-positive

    // An untouched account reports a zero balance, not an error.
    let cogs = account_balance(&pool, "5000").await?;
    assert_eq!(cogs.balance, 0);
    Ok(())
}

// --- Smoke test #4: idempotency — same key twice → one entry ----------------

#[sqlx::test(migrations = "../../migrations")]
async fn idempotent_replay(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let key = Uuid::new_v4();
    let lines = vec![line("1000", 5_000, 0), line("4000", 0, 5_000)];

    let first = post_journal_entry(&pool, &req(key, lines.clone())).await?;
    assert!(!first.replayed);

    let second = post_journal_entry(&pool, &req(key, lines)).await?;
    assert!(second.replayed);
    assert_eq!(first.entry_id, second.entry_id);

    assert_eq!(
        entry_count(&pool).await,
        1,
        "duplicate key must not create a second entry"
    );
    Ok(())
}

// --- Pre-check rejects unbalanced / malformed input, no DB write ------------

#[sqlx::test(migrations = "../../migrations")]
async fn unbalanced_rejected_before_db(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let err = post_journal_entry(
        &pool,
        &req(
            Uuid::new_v4(),
            vec![line("1000", 10_000, 0), line("4000", 0, 9_000)],
        ),
    )
    .await
    .unwrap_err();

    assert!(matches!(err, PostError::Domain(_)), "got {err:?}");
    assert_eq!(entry_count(&pool).await, 0);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn account_not_found(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let err = post_journal_entry(
        &pool,
        &req(
            Uuid::new_v4(),
            vec![line("1000", 1_000, 0), line("9999", 0, 1_000)],
        ),
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, PostError::AccountNotFound(c) if c == "9999"),
        "wrong error"
    );
    assert_eq!(entry_count(&pool).await, 0);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn journal_not_found(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let mut r = req(
        Uuid::new_v4(),
        vec![line("1000", 1_000, 0), line("4000", 0, 1_000)],
    );
    r.journal_code = "NOPE".into();
    let err = post_journal_entry(&pool, &r).await.unwrap_err();
    assert!(matches!(err, PostError::JournalNotFound(_)), "got {err:?}");
    assert_eq!(entry_count(&pool).await, 0);
    Ok(())
}

// --- Concurrency: same key, many racers → exactly one entry -----------------

#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_same_key_one_entry(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let key = Uuid::new_v4();
    let lines = vec![line("1000", 7_000, 0), line("4000", 0, 7_000)];

    let reqs: Vec<PostRequest> = (0..12).map(|_| req(key, lines.clone())).collect();
    let results = join_all(reqs.iter().map(|r| post_journal_entry(&pool, r))).await;

    let entry_ids: Vec<i64> = results
        .into_iter()
        .map(|r| r.expect("every racer should succeed").entry_id)
        .collect();

    assert_eq!(
        entry_count(&pool).await,
        1,
        "racing the same key must yield one entry"
    );
    assert!(
        entry_ids.windows(2).all(|w| w[0] == w[1]),
        "all racers must observe the same entry_id: {entry_ids:?}"
    );
    Ok(())
}

// --- Concurrency: distinct keys, same accounts → all post, balances sum -----

#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_distinct_keys_contended_accounts(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    const N: i64 = 15;
    let amount = 1_000;

    let reqs: Vec<PostRequest> = (0..N)
        .map(|_| {
            req(
                Uuid::new_v4(),
                vec![line("1000", amount, 0), line("4000", 0, amount)],
            )
        })
        .collect();
    let results = join_all(reqs.iter().map(|r| post_journal_entry(&pool, r))).await;

    for r in &results {
        assert!(r.is_ok(), "contended post failed: {:?}", r.as_ref().err());
    }
    assert_eq!(entry_count(&pool).await, N);

    let cash = account_balance(&pool, "1000").await?;
    assert_eq!(
        cash.balance,
        N * amount,
        "row locks + retry must not lose posts"
    );
    Ok(())
}

// --- The DB invariant itself: trigger rejects unbalanced at COMMIT ----------
// Posts that bypass the Rust pre-check (raw SQL) must still be rejected by the
// deferred CONSTRAINT TRIGGER. This is the credibility surface.

#[sqlx::test(migrations = "../../migrations")]
async fn db_trigger_rejects_single_line(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let jid: i64 = sqlx::query_scalar("SELECT id FROM journals WHERE code = 'GEN'")
        .fetch_one(&pool)
        .await?;
    let cash = account_id(&pool, "1000").await;

    let mut tx = pool.begin().await?;
    let eid: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (journal_id, entry_date) VALUES ($1, '2026-05-29') RETURNING id",
    )
    .bind(jid)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO journal_lines (entry_id, account_id, debit, credit) VALUES ($1, $2, 100, 0)",
    )
    .bind(eid)
    .bind(cash)
    .execute(&mut *tx)
    .await?;
    let commit = tx.commit().await;
    assert!(
        commit.is_err(),
        "single-line entry must be rejected at COMMIT"
    );
    assert_eq!(entry_count(&pool).await, 0);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn db_trigger_rejects_unbalanced(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let jid: i64 = sqlx::query_scalar("SELECT id FROM journals WHERE code = 'GEN'")
        .fetch_one(&pool)
        .await?;
    let cash = account_id(&pool, "1000").await;
    let sales = account_id(&pool, "4000").await;

    let mut tx = pool.begin().await?;
    let eid: i64 = sqlx::query_scalar(
        "INSERT INTO journal_entries (journal_id, entry_date) VALUES ($1, '2026-05-29') RETURNING id",
    )
    .bind(jid)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO journal_lines (entry_id, account_id, debit, credit) VALUES ($1, $2, 100, 0)",
    )
    .bind(eid)
    .bind(cash)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO journal_lines (entry_id, account_id, debit, credit) VALUES ($1, $2, 0, 90)",
    )
    .bind(eid)
    .bind(sales)
    .execute(&mut *tx)
    .await?;
    let commit = tx.commit().await;
    assert!(commit.is_err(), "Σdebit≠Σcredit must be rejected at COMMIT");
    assert_eq!(entry_count(&pool).await, 0);
    Ok(())
}

#[sqlx::test(migrations = "../../migrations")]
async fn db_trigger_rejects_empty_entry(pool: PgPool) -> TestResult {
    // A bare entry header with zero lines never fires the journal_lines trigger;
    // the journal_entries trigger must reject it at COMMIT.
    seed(&pool).await?;
    let jid: i64 = sqlx::query_scalar("SELECT id FROM journals WHERE code = 'GEN'")
        .fetch_one(&pool)
        .await?;

    let mut tx = pool.begin().await?;
    sqlx::query("INSERT INTO journal_entries (journal_id, entry_date) VALUES ($1, '2026-05-29')")
        .bind(jid)
        .execute(&mut *tx)
        .await?;
    let commit = tx.commit().await;
    assert!(
        commit.is_err(),
        "an entry with zero lines must be rejected at COMMIT"
    );
    assert_eq!(entry_count(&pool).await, 0);
    Ok(())
}

// --- The append-only invariant: UPDATE / DELETE on the ledger is forbidden --

#[sqlx::test(migrations = "../../migrations")]
async fn ledger_is_append_only(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let r = post_journal_entry(
        &pool,
        &req(
            Uuid::new_v4(),
            vec![line("1000", 3_000, 0), line("4000", 0, 3_000)],
        ),
    )
    .await?;

    let del = sqlx::query("DELETE FROM journal_entries WHERE id = $1")
        .bind(r.entry_id)
        .execute(&pool)
        .await;
    assert!(del.is_err(), "DELETE on journal_entries must be forbidden");

    let upd = sqlx::query("UPDATE journal_lines SET debit = 1 WHERE entry_id = $1")
        .bind(r.entry_id)
        .execute(&pool)
        .await;
    assert!(upd.is_err(), "UPDATE on journal_lines must be forbidden");
    Ok(())
}

// --- Determinism: balances are a pure projection of the journal log ---------
// Smoke test #2 (proxy): aggregate the log independently in Rust and assert it
// equals the SQL-computed balance. Full fresh-DB event replay lands with
// ol-events.

#[sqlx::test(migrations = "../../migrations")]
async fn balances_are_projection_of_log(pool: PgPool) -> TestResult {
    seed(&pool).await?;
    let posts = [
        vec![line("1000", 10_000, 0), line("4000", 0, 10_000)],
        vec![line("5000", 4_000, 0), line("1000", 0, 4_000)],
        vec![line("1000", 2_500, 0), line("4000", 0, 2_500)],
    ];
    for lines in posts {
        post_journal_entry(&pool, &req(Uuid::new_v4(), lines)).await?;
    }

    // Independent replay: fold the raw lines per account code.
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT a.code, l.debit, l.credit FROM journal_lines l \
         JOIN accounts a ON a.id = l.account_id",
    )
    .fetch_all(&pool)
    .await?;
    let mut replay: std::collections::HashMap<String, (i64, i64)> =
        std::collections::HashMap::new();
    for (code, d, c) in rows {
        let e = replay.entry(code).or_insert((0, 0));
        e.0 += d;
        e.1 += c;
    }

    for code in ["1000", "4000", "5000"] {
        let b = account_balance(&pool, code).await?;
        let (rd, rc) = replay.get(code).copied().unwrap_or((0, 0));
        assert_eq!(b.debits, rd, "debits mismatch for {code}");
        assert_eq!(b.credits, rc, "credits mismatch for {code}");
    }
    // Cash: 10_000 + 2_500 debit, 4_000 credit → 8_500 net (asset, debit-positive)
    assert_eq!(account_balance(&pool, "1000").await?.balance, 8_500);
    Ok(())
}
