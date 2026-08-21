//! Integration test pinning transition-guard enforcement in ol-engine.
//!
//! The `customer_invoice` process declares `guards: [lines_nonempty, totals_balance]`
//! on its `draft -> posted` transition (`capability: post_invoice`). Guards are
//! currently parsed from YAML but never evaluated. This test documents the
//! intended contract so the failure is visible and the fix has a regression pin.
//!
//! Guard contract assumed by this test:
//!   - `lines_nonempty`: `context.lines` exists, is an array, and has at least one
//!     element.
//!   - `totals_balance`: the sum of `amount` on every object in `context.lines`
//!     equals `context.total`.

use ol_engine::{AdvanceInput, advance_instance, get_instance, start_instance};
use ol_ledger::account_balance;
use sqlx::PgPool;
use std::collections::HashMap;

const PROCESSES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../processes");

fn today() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 6, 14).unwrap()
}

/// GIVEN a `customer_invoice` instance in `draft` whose guard context is invalid
///       (non-empty lines but the line total does not equal `context.total`)
/// WHEN `post_invoice` is called with otherwise valid posting amounts
/// THEN the advance is REFUSED with an error, the instance stays in `draft`,
///      no GL entry is posted, and the step log has no `post_invoice` row.
///
/// GIVEN the same instance after patching `context` so lines sum to `total`
/// WHEN `post_invoice` is called again with the same amounts
/// THEN the advance SUCCEEDS, the instance moves exactly one step to `posted`,
///      and the GL reflects the posting rule (DR AR, CR sales_revenue + tax_payable).
#[sqlx::test(migrations = "../../migrations")]
async fn customer_invoice_post_invoice_guard_is_enforced(pool: PgPool) {
    // SAFETY: single-threaded test setup; no other threads read this var yet.
    unsafe { std::env::set_var("PROCESSES_DIR", PROCESSES_DIR) };

    // -------------------------------------------------------------------------
    // Setup
    // -------------------------------------------------------------------------
    let total: i64 = 60_000; // €600.00
    let sales: i64 = 50_000; // €500.00
    let tax: i64 = 10_000; // €100.00
    assert_eq!(sales + tax, total);

    let bad_context = serde_json::json!({
        "customer": "Acme Corp",
        "lines": [{ "description": "Widget", "amount": 50_000 }],
        "total": total
    });

    let instance = start_instance(
        &pool,
        "customer_invoice",
        Some("INV-001".to_string()),
        bad_context,
    )
    .await
    .expect("start customer_invoice instance");

    assert_eq!(instance.current_state, "draft");
    assert_eq!(instance.status, "active");
    let id = instance.id;

    let mut amounts = HashMap::new();
    amounts.insert("amount".to_string(), total);
    amounts.insert("sales_revenue".to_string(), sales);
    amounts.insert("tax_payable".to_string(), tax);

    // -------------------------------------------------------------------------
    // Case (1): guard NOT satisfied -> refused, state unchanged, no GL post.
    // -------------------------------------------------------------------------
    let first = advance_instance(
        &pool,
        id,
        "post_invoice",
        "test_actor",
        AdvanceInput {
            amounts: amounts.clone(),
            context_patch: serde_json::json!({}),
            entry_date: today(),
        },
    )
    .await;

    assert!(
        first.is_err(),
        "post_invoice with unbalanced line totals must be refused by the guards; got Ok: {first:?}"
    );

    let after_refusal = get_instance(&pool, id)
        .await
        .expect("get instance after refusal");
    assert_eq!(
        after_refusal.instance.current_state, "draft",
        "instance must remain in draft after guard refusal"
    );
    assert_eq!(
        after_refusal.instance.status, "active",
        "instance status must remain active"
    );
    assert_eq!(
        after_refusal.steps.len(),
        1,
        "step log must contain only the start row after refusal"
    );
    assert!(
        after_refusal
            .steps
            .iter()
            .all(|s| s.capability != "post_invoice"),
        "no post_invoice step log row must be written when the guard refuses"
    );

    // No ledger entry was posted.
    let ar_before = account_balance(&pool, "1100").await.expect("AR balance");
    assert_eq!(
        ar_before.debits, 0,
        "AR must not be debited when guard refuses"
    );
    let sales_before = account_balance(&pool, "4000").await.expect("sales balance");
    assert_eq!(
        sales_before.credits, 0,
        "sales_revenue must not be credited when guard refuses"
    );
    let tax_before = account_balance(&pool, "2100").await.expect("tax balance");
    assert_eq!(
        tax_before.credits, 0,
        "tax_payable must not be credited when guard refuses"
    );

    // -------------------------------------------------------------------------
    // Case (2): guard satisfied -> succeeds and advances exactly one step.
    // -------------------------------------------------------------------------
    let good_patch = serde_json::json!({
        "lines": [
            { "description": "Net item", "amount": sales },
            { "description": "Tax item", "amount": tax }
        ],
        "total": total
    });

    let second = advance_instance(
        &pool,
        id,
        "post_invoice",
        "test_actor",
        AdvanceInput {
            amounts: amounts.clone(),
            context_patch: good_patch,
            entry_date: today(),
        },
    )
    .await;

    let advanced = second.expect("post_invoice must succeed when guards are satisfied");
    assert_eq!(
        advanced.current_state, "posted",
        "instance must advance exactly one step to posted"
    );
    assert_eq!(
        advanced.status, "active",
        "posted is not terminal; status stays active"
    );

    let after_success = get_instance(&pool, id)
        .await
        .expect("get instance after success");
    assert_eq!(
        after_success.steps.len(),
        2,
        "step log must contain start + post_invoice rows after success"
    );
    let post_steps: Vec<_> = after_success
        .steps
        .iter()
        .filter(|s| s.capability == "post_invoice")
        .collect();
    assert_eq!(post_steps.len(), 1, "exactly one post_invoice step row");
    assert_eq!(post_steps[0].from_state, "draft");
    assert_eq!(post_steps[0].to_state, "posted");
    assert!(
        post_steps[0].entry_id.is_some(),
        "post_invoice step must reference a journal entry"
    );

    // GL reflects the posting rule.
    let ar_after = account_balance(&pool, "1100").await.expect("AR balance");
    assert_eq!(
        ar_after.debits, total,
        "AR must be debited by the invoice total"
    );
    let sales_after = account_balance(&pool, "4000").await.expect("sales balance");
    assert_eq!(
        sales_after.credits, sales,
        "sales_revenue must be credited by the net amount"
    );
    let tax_after = account_balance(&pool, "2100").await.expect("tax balance");
    assert_eq!(
        tax_after.credits, tax,
        "tax_payable must be credited by the tax amount"
    );
}
