//! Integration tests for GET /reports/trial-balance and
//! GET /reports/subledger-reconciliation.
//!
//! Each test gets a fresh migrated database via `#[sqlx::test]`.
//! Migration `0002_seed_chart_of_accounts.sql` populates the COA and the `GEN`
//! journal, so accounts 1000/1100/1200/2000/2100/3000/4000/5000 and journal `GEN`
//! are always present.

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

/// Post a balanced journal entry via the REST handler. Panics on failure.
async fn post_entry(pool: PgPool, key: &str, lines: Value) {
    let payload = json!({
        "idempotency_key": key,
        "journal_code": "GEN",
        "entry_date": "2025-06-01",
        "memo": "reports test",
        "actor": "test",
        "lines": lines
    });
    let (status, body) = post_json(pool, "/journal-entries", payload).await;
    assert!(
        status == StatusCode::CREATED || status == StatusCode::OK,
        "post_entry failed: status={status} body={body}"
    );
}

// ─── GET /reports/trial-balance ───────────────────────────────────────────────

/// Empty ledger: no balance rows exist, so the trial balance returns zero lines
/// but the totals are still zero and balanced=true.
///
/// GIVEN a freshly migrated database with no journal entries
/// WHEN  GET /reports/trial-balance is called
/// THEN  it returns 200, lines is empty (no currency rows), and balanced=true
#[sqlx::test(migrations = "../../migrations")]
async fn trial_balance_empty_ledger_is_balanced(pool: PgPool) {
    let (status, body) = get(pool, "/reports/trial-balance").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    assert_eq!(body["total_debits_cents"], 0, "no debits yet");
    assert_eq!(body["total_credits_cents"], 0, "no credits yet");
    assert_eq!(body["balanced"], true, "empty ledger must be balanced");

    // Per-currency grouping uses INNER JOIN on account_balances: accounts with
    // no posted lines have no balance rows and therefore no trial-balance line.
    let lines = body["lines"].as_array().expect("lines must be array");
    assert!(
        lines.is_empty(),
        "no balance rows exist — lines must be empty, got {lines:?}"
    );
}

/// After a balanced journal entry the trial balance remains balanced and
/// the debits/credits move to the correct accounts with a currency field.
///
/// GIVEN Dr 1000 Cash 50_000 / Cr 4000 Revenue 50_000 is posted (currency EUR)
/// WHEN  GET /reports/trial-balance is called
/// THEN  total_debits == total_credits == 50_000, balanced=true,
///       1000 EUR shows debits=50_000 balance=50_000 currency="EUR",
///       4000 EUR shows credits=50_000 balance=50_000 currency="EUR"
#[sqlx::test(migrations = "../../migrations")]
async fn trial_balance_after_entry_reflects_amounts(pool: PgPool) {
    // Dr 1000 Cash / Cr 4000 Revenue (currency defaults to EUR)
    post_entry(
        pool.clone(),
        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
        json!([
            { "account_code": "1000", "debit": 50_000, "credit": 0 },
            { "account_code": "4000", "debit": 0, "credit": 50_000 }
        ]),
    )
    .await;

    let (status, body) = get(pool, "/reports/trial-balance").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    assert_eq!(body["total_debits_cents"], 50_000, "total debits");
    assert_eq!(body["total_credits_cents"], 50_000, "total credits");
    assert_eq!(
        body["balanced"], true,
        "must be balanced after balanced entry"
    );

    let lines = body["lines"].as_array().expect("lines must be array");

    let cash = lines
        .iter()
        .find(|l| l["code"] == "1000" && l["currency"] == "EUR")
        .expect("1000 Cash EUR must be in trial balance");
    assert_eq!(cash["debits_cents"], 50_000, "cash debits");
    assert_eq!(cash["credits_cents"], 0, "cash credits");
    // asset: balance = debits - credits
    assert_eq!(cash["balance_cents"], 50_000, "cash balance (asset)");
    assert_eq!(cash["account_type"], "asset", "1000 is asset");
    assert_eq!(cash["currency"], "EUR", "line currency must be EUR");

    let revenue = lines
        .iter()
        .find(|l| l["code"] == "4000" && l["currency"] == "EUR")
        .expect("4000 Revenue EUR must be in trial balance");
    assert_eq!(revenue["debits_cents"], 0, "revenue debits");
    assert_eq!(revenue["credits_cents"], 50_000, "revenue credits");
    // income: balance = credits - debits
    assert_eq!(revenue["balance_cents"], 50_000, "revenue balance (income)");
    assert_eq!(revenue["account_type"], "income", "4000 is income");
    assert_eq!(revenue["currency"], "EUR", "line currency must be EUR");
}

/// Lines are returned ordered by account code then currency.
///
/// GIVEN two balanced entries touching 1000/2000 (EUR) and 1100/4000 (EUR)
/// WHEN  GET /reports/trial-balance is called
/// THEN  lines appear in ascending (code, currency) order
#[sqlx::test(migrations = "../../migrations")]
async fn trial_balance_lines_ordered_by_code(pool: PgPool) {
    // Seed two entries so multiple accounts have balance rows.
    // Entry 1: Dr 1000 Cash / Cr 2000 AP (both EUR)
    post_entry(
        pool.clone(),
        "cccccccc-cccc-cccc-cccc-cccccccccccc",
        json!([
            { "account_code": "1000", "debit": 10_000, "credit": 0 },
            { "account_code": "2000", "debit": 0, "credit": 10_000 }
        ]),
    )
    .await;
    // Entry 2: Dr 1100 AR / Cr 4000 Revenue (both EUR)
    post_entry(
        pool.clone(),
        "dddddddd-dddd-dddd-dddd-dddddddddddd",
        json!([
            { "account_code": "1100", "debit": 5_000, "credit": 0 },
            { "account_code": "4000", "debit": 0, "credit": 5_000 }
        ]),
    )
    .await;

    let (status, body) = get(pool, "/reports/trial-balance").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let lines = body["lines"].as_array().expect("lines must be array");
    assert!(!lines.is_empty(), "must have lines after seeding entries");

    // Build (code, currency) pairs and verify they are already sorted.
    let pairs: Vec<(String, String)> = lines
        .iter()
        .map(|l| {
            (
                l["code"].as_str().expect("code must be string").to_owned(),
                l["currency"]
                    .as_str()
                    .expect("currency must be string")
                    .to_owned(),
            )
        })
        .collect();

    let mut sorted = pairs.clone();
    sorted.sort();
    assert_eq!(pairs, sorted, "lines must be sorted by (code, currency)");
}

// ─── GET /reports/subledger-reconciliation ────────────────────────────────────

/// Empty AR/AP: both sections show zero and reconcile.
///
/// GIVEN no invoices or bills exist
/// WHEN  GET /reports/subledger-reconciliation is called
/// THEN  AR and AP both show control=0, subledger=0, difference=0
#[sqlx::test(migrations = "../../migrations")]
async fn subledger_reconciliation_empty_is_zero(pool: PgPool) {
    let (status, body) = get(pool, "/reports/subledger-reconciliation").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    assert_eq!(body["ar"]["control_balance_cents"], 0, "AR control");
    assert_eq!(body["ar"]["subledger_total_cents"], 0, "AR subledger");
    assert_eq!(body["ar"]["difference_cents"], 0, "AR difference");

    assert_eq!(body["ap"]["control_balance_cents"], 0, "AP control");
    assert_eq!(body["ap"]["subledger_total_cents"], 0, "AP subledger");
    assert_eq!(body["ap"]["difference_cents"], 0, "AP difference");
}

/// AR subledger total matches open invoices; difference is zero after posting
/// the AR control account.
///
/// GIVEN Dr 1100 AR 30_000 / Cr 4000 Revenue 30_000 is posted in GL
///   AND an open invoice for 30_000 exists in the invoices table
/// WHEN  GET /reports/subledger-reconciliation is called
/// THEN  AR control_balance=30_000, subledger_total=30_000, difference=0
#[sqlx::test(migrations = "../../migrations")]
async fn subledger_reconciliation_ar_reconciles_after_invoice_and_entry(pool: PgPool) {
    // Seed a customer and an open invoice for 30_000 cents.
    sqlx::query!("INSERT INTO parties (kind, name) VALUES ('customer', 'Test Corp')")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query!(
        r#"
        INSERT INTO invoices (customer_id, state, due_date, total)
        SELECT id, 'posted', CURRENT_DATE + INTERVAL '30 days', 30000
          FROM parties WHERE name = 'Test Corp'
        "#
    )
    .execute(&pool)
    .await
    .unwrap();

    // Post the matching GL entry: Dr 1100 AR / Cr 4000 Revenue
    post_entry(
        pool.clone(),
        "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
        json!([
            { "account_code": "1100", "debit": 30_000, "credit": 0 },
            { "account_code": "4000", "debit": 0, "credit": 30_000 }
        ]),
    )
    .await;

    let (status, body) = get(pool, "/reports/subledger-reconciliation").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    // AR control: 1100 is asset → debit-positive → 30_000
    assert_eq!(
        body["ar"]["control_balance_cents"], 30_000,
        "AR control balance"
    );
    assert_eq!(
        body["ar"]["subledger_total_cents"], 30_000,
        "AR subledger total"
    );
    assert_eq!(body["ar"]["difference_cents"], 0, "AR difference must be 0");
}

/// Paid invoices are excluded from the AR sub-ledger total.
///
/// GIVEN two invoices: one open (20_000) and one paid (10_000)
/// WHEN  GET /reports/subledger-reconciliation is called
/// THEN  AR subledger_total == 20_000 (paid invoice excluded)
#[sqlx::test(migrations = "../../migrations")]
async fn subledger_reconciliation_ar_excludes_paid_invoices(pool: PgPool) {
    sqlx::query!("INSERT INTO parties (kind, name) VALUES ('customer', 'Mixed State Corp')")
        .execute(&pool)
        .await
        .unwrap();

    // Open invoice: 20_000
    sqlx::query!(
        r#"
        INSERT INTO invoices (customer_id, state, total)
        SELECT id, 'posted', 20000 FROM parties WHERE name = 'Mixed State Corp'
        "#
    )
    .execute(&pool)
    .await
    .unwrap();

    // Paid invoice: 10_000 — must be excluded
    sqlx::query!(
        r#"
        INSERT INTO invoices (customer_id, state, total)
        SELECT id, 'paid', 10000 FROM parties WHERE name = 'Mixed State Corp'
        "#
    )
    .execute(&pool)
    .await
    .unwrap();

    let (status, body) = get(pool, "/reports/subledger-reconciliation").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    assert_eq!(
        body["ar"]["subledger_total_cents"], 20_000,
        "paid invoice must be excluded; only open invoice counts"
    );
}

/// Draft invoices are excluded from the AR sub-ledger total.
///
/// GIVEN a draft invoice for 15_000 exists (never posted to GL)
/// WHEN  GET /reports/subledger-reconciliation is called
/// THEN  AR subledger_total == 0 (draft invoice must not count)
#[sqlx::test(migrations = "../../migrations")]
async fn subledger_reconciliation_ar_excludes_draft_invoices(pool: PgPool) {
    sqlx::query!("INSERT INTO parties (kind, name) VALUES ('customer', 'Draft Co')")
        .execute(&pool)
        .await
        .unwrap();

    // Draft invoice: 15_000 — has NOT been posted to GL control account 1100.
    sqlx::query!(
        r#"
        INSERT INTO invoices (customer_id, state, total)
        SELECT id, 'draft', 15000 FROM parties WHERE name = 'Draft Co'
        "#
    )
    .execute(&pool)
    .await
    .unwrap();

    let (status, body) = get(pool, "/reports/subledger-reconciliation").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    assert_eq!(
        body["ar"]["subledger_total_cents"], 0,
        "draft invoice must not count toward AR subledger total"
    );
    // Control is also 0 (no GL entry posted) → difference is 0.
    assert_eq!(
        body["ar"]["difference_cents"], 0,
        "AR difference must be 0 when draft invoice is excluded"
    );
}

/// AP reconciles after seeding an open posted bill and its GL entry.
///
/// GIVEN Dr 5000 Expense 45_000 / Cr 2000 AP 45_000 is posted in GL
///   AND an open bill for 45_000 exists in the bills table (state=posted)
/// WHEN  GET /reports/subledger-reconciliation is called
/// THEN  AP control_balance=45_000, subledger_total=45_000, difference=0
#[sqlx::test(migrations = "../../migrations")]
async fn subledger_reconciliation_ap_reconciles_after_bill_and_entry(pool: PgPool) {
    sqlx::query!("INSERT INTO parties (kind, name) VALUES ('vendor', 'Acme Supplies')")
        .execute(&pool)
        .await
        .unwrap();

    // Open posted bill for 45_000 cents.
    sqlx::query!(
        r#"
        INSERT INTO bills (vendor_id, state, total)
        SELECT id, 'posted', 45000
          FROM parties WHERE name = 'Acme Supplies'
        "#
    )
    .execute(&pool)
    .await
    .unwrap();

    // Post matching GL entry: Dr 5000 Expense / Cr 2000 AP
    post_entry(
        pool.clone(),
        "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee",
        json!([
            { "account_code": "5000", "debit": 45_000, "credit": 0 },
            { "account_code": "2000", "debit": 0, "credit": 45_000 }
        ]),
    )
    .await;

    let (status, body) = get(pool, "/reports/subledger-reconciliation").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    // AP control: 2000 is liability → credit-positive → 45_000
    assert_eq!(
        body["ap"]["control_balance_cents"], 45_000,
        "AP control balance"
    );
    assert_eq!(
        body["ap"]["subledger_total_cents"], 45_000,
        "AP subledger total"
    );
    assert_eq!(body["ap"]["difference_cents"], 0, "AP difference must be 0");
}

/// Trial balance per-currency filter returns only the requested currency,
/// and each line carries a `currency` field.
///
/// GIVEN Dr 1000 Cash 20_000 / Cr 4000 Revenue 20_000 posted in EUR
/// WHEN  GET /reports/trial-balance?currency=EUR is called
/// THEN  lines are present with currency="EUR"
/// WHEN  GET /reports/trial-balance?currency=USD is called
/// THEN  lines is empty (no USD postings)
#[sqlx::test(migrations = "../../migrations")]
async fn trial_balance_per_currency_filter(pool: PgPool) {
    // Post EUR entry
    post_entry(
        pool.clone(),
        "ffffffff-ffff-ffff-ffff-ffffffffffff",
        json!([
            { "account_code": "1000", "debit": 20_000, "credit": 0, "currency": "EUR" },
            { "account_code": "4000", "debit": 0, "credit": 20_000, "currency": "EUR" }
        ]),
    )
    .await;

    // Filter for EUR — expect lines with currency="EUR"
    let (status, body) = get(pool.clone(), "/reports/trial-balance?currency=EUR").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    let lines = body["lines"].as_array().expect("lines must be array");
    assert!(!lines.is_empty(), "EUR filter must return lines");
    for line in lines {
        assert_eq!(
            line["currency"], "EUR",
            "all lines must have currency=EUR when filtered"
        );
    }
    assert_eq!(body["balanced"], true, "EUR subset must balance");

    // Filter for USD — no USD postings → empty lines, balanced=true
    let (status2, body2) = get(pool, "/reports/trial-balance?currency=USD").await;
    assert_eq!(status2, StatusCode::OK, "body2={body2}");

    let lines2 = body2["lines"].as_array().expect("lines must be array");
    assert!(
        lines2.is_empty(),
        "USD filter must return no lines when only EUR postings exist"
    );
    assert_eq!(body2["balanced"], true, "empty USD filter must be balanced");
    assert_eq!(body2["total_debits_cents"], 0);
    assert_eq!(body2["total_credits_cents"], 0);
}
