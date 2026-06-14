//! Integration tests for GET /metrics/overview and GET /metrics/ar-aging.
//!
//! Each test gets a fresh migrated database via `#[sqlx::test]`.
//! Migration `0002_seed_chart_of_accounts.sql` populates the COA and the `GEN`
//! journal so accounts 1000/1100/2000/4000/5000 and journal `GEN` are always
//! present.
//!
//! Journal entries are posted through the REST handler to exercise the full
//! stack (trigger, balance cache, aggregate query).

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use ol_api::app;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

// ─── Helpers ──────────────────────────────────────────────────────────────────

async fn get(pool: PgPool, path: &str) -> (StatusCode, Value) {
    let router = app(pool);
    let req = Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

async fn post_json(pool: PgPool, path: &str, payload: Value) -> (StatusCode, Value) {
    let router = app(pool);
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&payload).unwrap()))
        .unwrap();
    let resp = router.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

/// Post a balanced journal entry. Panics if the post fails.
async fn post_entry(pool: PgPool, key: &str, lines: Value) {
    let payload = json!({
        "idempotency_key": key,
        "journal_code": "GEN",
        "entry_date": "2025-06-01",
        "memo": "metrics test",
        "actor": "test",
        "lines": lines
    });
    let (status, body) = post_json(pool, "/journal-entries", payload).await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "post_entry failed: status={status} body={body}"
    );
}

// ─── GET /metrics/overview ────────────────────────────────────────────────────

/// Empty ledger (no journal entries) returns zero for all money fields and
/// correct account_count from the seeded COA.
#[sqlx::test(migrations = "../../migrations")]
async fn overview_empty_ledger_returns_zeros(pool: PgPool) {
    let (status, body) = get(pool, "/metrics/overview").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    assert_eq!(body["cash_cents"], 0, "cash_cents must be 0");
    assert_eq!(body["revenue_cents"], 0, "revenue_cents must be 0");
    assert_eq!(body["expenses_cents"], 0, "expenses_cents must be 0");
    assert_eq!(body["ar_cents"], 0, "ar_cents must be 0");
    assert_eq!(body["ap_cents"], 0, "ap_cents must be 0");
    // Migration 0002 seeds 8 accounts.
    assert_eq!(
        body["account_count"], 8,
        "account_count must match seeded COA"
    );
}

/// After posting cash-received-from-revenue entry (Dr 1000 Cash / Cr 4000 Revenue),
/// cash_cents and revenue_cents reflect the posted amounts.
#[sqlx::test(migrations = "../../migrations")]
async fn overview_cash_and_revenue_after_entry(pool: PgPool) {
    // Dr 1000 Cash 50_000 / Cr 4000 Revenue 50_000
    post_entry(
        pool.clone(),
        "11111111-1111-1111-1111-111111111001",
        json!([
            { "account_code": "1000", "debit": 50_000, "credit": 0 },
            { "account_code": "4000", "debit": 0, "credit": 50_000 }
        ]),
    )
    .await;

    let (status, body) = get(pool, "/metrics/overview").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    // 1000 is asset: balance = debits - credits = 50_000
    assert_eq!(body["cash_cents"], 50_000, "cash_cents");
    // 4000 is income: balance = credits - debits = 50_000
    assert_eq!(body["revenue_cents"], 50_000, "revenue_cents");
    assert_eq!(body["expenses_cents"], 0, "no expenses posted");
    assert_eq!(body["ar_cents"], 0, "no AR posted");
}

/// ar_cents reflects the balance of account 1100 specifically.
#[sqlx::test(migrations = "../../migrations")]
async fn overview_ar_cents_reflects_account_1100(pool: PgPool) {
    // Dr 1100 AR 30_000 / Cr 4000 Revenue 30_000 (typical invoice post)
    post_entry(
        pool.clone(),
        "22222222-2222-2222-2222-222222222001",
        json!([
            { "account_code": "1100", "debit": 30_000, "credit": 0 },
            { "account_code": "4000", "debit": 0, "credit": 30_000 }
        ]),
    )
    .await;

    let (status, body) = get(pool, "/metrics/overview").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    // ar_cents = balance of 1100 = debits – credits = 30_000
    assert_eq!(body["ar_cents"], 30_000, "ar_cents");
    // cash_cents includes ALL asset accounts; 1100 is also asset
    assert_eq!(body["cash_cents"], 30_000, "cash_cents includes 1100");
}

/// expenses_cents reflects expense account balances (debit-positive).
#[sqlx::test(migrations = "../../migrations")]
async fn overview_expenses_after_cogs_entry(pool: PgPool) {
    // Dr 5000 COGS 12_000 / Cr 1200 Inventory 12_000
    post_entry(
        pool.clone(),
        "33333333-3333-3333-3333-333333333001",
        json!([
            { "account_code": "5000", "debit": 12_000, "credit": 0 },
            { "account_code": "1200", "debit": 0, "credit": 12_000 }
        ]),
    )
    .await;

    let (status, body) = get(pool, "/metrics/overview").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    assert_eq!(body["expenses_cents"], 12_000, "expenses_cents");
    // 1200 Inventory is asset with a credit → balance = 0 - 12_000 = -12_000,
    // so cash_cents (sum of asset balances) should be -12_000
    assert_eq!(
        body["cash_cents"], -12_000,
        "cash_cents reflects negative inventory balance"
    );
}

// ─── GET /metrics/ar-aging ────────────────────────────────────────────────────

/// Empty invoices table: all 5 buckets present with total_cents = 0.
#[sqlx::test(migrations = "../../migrations")]
async fn ar_aging_empty_returns_five_zero_buckets(pool: PgPool) {
    let (status, body) = get(pool, "/metrics/ar-aging").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let buckets = body["buckets"].as_array().expect("buckets must be array");
    assert_eq!(buckets.len(), 5, "must return exactly 5 buckets");

    let labels: Vec<_> = buckets
        .iter()
        .map(|b| b["label"].as_str().unwrap())
        .collect();
    assert_eq!(
        labels,
        vec!["current", "1-30", "31-60", "61-90", "90+"],
        "bucket order"
    );

    for bucket in buckets {
        assert_eq!(
            bucket["total_cents"], 0,
            "all buckets must be zero: {bucket}"
        );
    }
}

/// An invoice with due_date in the future lands in the 'current' bucket.
#[sqlx::test(migrations = "../../migrations")]
async fn ar_aging_future_due_date_lands_in_current(pool: PgPool) {
    // Insert a customer (required FK) and an open invoice with future due_date.
    sqlx::query!("INSERT INTO parties (kind, name) VALUES ('customer', 'Test Corp')")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query!(
        r#"
        INSERT INTO invoices (customer_id, state, due_date, total)
        SELECT id, 'posted', CURRENT_DATE + INTERVAL '30 days', 10000
          FROM parties WHERE name = 'Test Corp'
        "#
    )
    .execute(&pool)
    .await
    .unwrap();

    let (status, body) = get(pool, "/metrics/ar-aging").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let buckets = body["buckets"].as_array().unwrap();
    let current = &buckets[0];
    assert_eq!(current["label"], "current", "first bucket label");
    assert_eq!(current["total_cents"], 10_000, "invoice lands in current");

    // All other buckets must be zero.
    for bucket in &buckets[1..] {
        assert_eq!(
            bucket["total_cents"], 0,
            "non-current bucket must be zero: {bucket}"
        );
    }
}

/// An invoice 45 days overdue lands in the '31-60' bucket.
#[sqlx::test(migrations = "../../migrations")]
async fn ar_aging_overdue_45_days_lands_in_31_60(pool: PgPool) {
    sqlx::query!("INSERT INTO parties (kind, name) VALUES ('customer', 'Late Payer Inc')")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query!(
        r#"
        INSERT INTO invoices (customer_id, state, due_date, total)
        SELECT id, 'posted', CURRENT_DATE - INTERVAL '45 days', 7500
          FROM parties WHERE name = 'Late Payer Inc'
        "#
    )
    .execute(&pool)
    .await
    .unwrap();

    let (status, body) = get(pool, "/metrics/ar-aging").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let buckets = body["buckets"].as_array().unwrap();
    // bucket index: 0=current, 1=1-30, 2=31-60, 3=61-90, 4=90+
    let bucket_31_60 = &buckets[2];
    assert_eq!(bucket_31_60["label"], "31-60", "label check");
    assert_eq!(bucket_31_60["total_cents"], 7_500, "invoice lands in 31-60");

    assert_eq!(buckets[0]["total_cents"], 0, "current must be zero");
    assert_eq!(buckets[1]["total_cents"], 0, "1-30 must be zero");
    assert_eq!(buckets[3]["total_cents"], 0, "61-90 must be zero");
    assert_eq!(buckets[4]["total_cents"], 0, "90+ must be zero");
}

/// A paid invoice is excluded from aging.
#[sqlx::test(migrations = "../../migrations")]
async fn ar_aging_paid_invoice_excluded(pool: PgPool) {
    sqlx::query!("INSERT INTO parties (kind, name) VALUES ('customer', 'Paid Up LLC')")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query!(
        r#"
        INSERT INTO invoices (customer_id, state, due_date, total)
        SELECT id, 'paid', CURRENT_DATE - INTERVAL '10 days', 5000
          FROM parties WHERE name = 'Paid Up LLC'
        "#
    )
    .execute(&pool)
    .await
    .unwrap();

    let (status, body) = get(pool, "/metrics/ar-aging").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let buckets = body["buckets"].as_array().unwrap();
    for bucket in buckets {
        assert_eq!(
            bucket["total_cents"], 0,
            "paid invoice must be excluded: {bucket}"
        );
    }
}

/// An invoice with NULL due_date lands in the 'current' bucket.
#[sqlx::test(migrations = "../../migrations")]
async fn ar_aging_null_due_date_lands_in_current(pool: PgPool) {
    sqlx::query!("INSERT INTO parties (kind, name) VALUES ('customer', 'No Due Date Co')")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query!(
        r#"
        INSERT INTO invoices (customer_id, state, due_date, total)
        SELECT id, 'draft', NULL, 2500
          FROM parties WHERE name = 'No Due Date Co'
        "#
    )
    .execute(&pool)
    .await
    .unwrap();

    let (status, body) = get(pool, "/metrics/ar-aging").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let buckets = body["buckets"].as_array().unwrap();
    assert_eq!(
        buckets[0]["total_cents"], 2_500,
        "null due_date lands in current"
    );
}

/// An invoice more than 90 days overdue lands in the '90+' bucket.
#[sqlx::test(migrations = "../../migrations")]
async fn ar_aging_overdue_120_days_lands_in_90_plus(pool: PgPool) {
    sqlx::query!("INSERT INTO parties (kind, name) VALUES ('customer', 'Ancient Debt GmbH')")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query!(
        r#"
        INSERT INTO invoices (customer_id, state, due_date, total)
        SELECT id, 'posted', CURRENT_DATE - INTERVAL '120 days', 99_999
          FROM parties WHERE name = 'Ancient Debt GmbH'
        "#
    )
    .execute(&pool)
    .await
    .unwrap();

    let (status, body) = get(pool, "/metrics/ar-aging").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let buckets = body["buckets"].as_array().unwrap();
    let bucket_90_plus = &buckets[4];
    assert_eq!(bucket_90_plus["label"], "90+", "label check");
    assert_eq!(
        bucket_90_plus["total_cents"], 99_999,
        "invoice lands in 90+"
    );
}
