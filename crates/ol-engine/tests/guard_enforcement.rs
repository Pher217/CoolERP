//! Integration test pinning transition-guard enforcement in ol-engine.
//!
//! The `customer_invoice` process declares `guards: [lines_nonempty, totals_balance]`
//! on its `draft -> posted` transition (`capability: post_invoice`). This test file
//! documents the intended guard contract and pins every refusal edge that an
//! adversarial review found missing.
//!
//! Guard contract assumed by this file:
//!   - `lines_nonempty`: `context.lines` exists, is an array, and has at least one
//!     element. Missing `lines` is a refusal (fail-closed), not a pass.
//!   - `totals_balance`: the sum of `amount` on every object in `context.lines`
//!     equals `context.total`. Missing `total` is a refusal (fail-closed).
//!   - Any guard name the engine does not implement is refused (fail-closed).

use ol_engine::{AdvanceInput, advance_instance, get_instance, start_instance};
use ol_ledger::account_balance;
use sqlx::PgPool;
use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

/// Processes directory shared by every test in this file.
///
/// It contains a copy of `processes/customer_invoice.yaml` plus a test-only
/// `unknown_guard_invoice.yaml` definition. Using one shared directory avoids
/// races on the `PROCESSES_DIR` environment variable across concurrent tests in
/// this binary.
fn test_processes_dir() -> &'static Path {
    static DIR: OnceLock<std::path::PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../processes");
        let dir =
            std::env::temp_dir().join(format!("ol-engine-guard-tests-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create shared test processes dir");
        std::fs::copy(
            base.join("customer_invoice.yaml"),
            dir.join("customer_invoice.yaml"),
        )
        .expect("copy customer_invoice.yaml into test processes dir");
        std::fs::write(
            dir.join("unknown_guard_invoice.yaml"),
            r#"process: unknown_guard_invoice
states: [draft, posted]
transitions:
  - from: draft
    to: posted
    capability: post_invoice
    guards: [this_guard_is_not_implemented]
"#,
        )
        .expect("write unknown_guard_invoice.yaml into test processes dir");
        dir
    })
}

fn today() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 6, 14).unwrap()
}

fn sample_posting_amounts() -> HashMap<String, i64> {
    HashMap::from([
        ("amount".to_string(), 60_000),
        ("sales_revenue".to_string(), 50_000),
        ("tax_payable".to_string(), 10_000),
    ])
}

fn set_processes_dir() {
    // SAFETY: all tests in this file set `PROCESSES_DIR` to the same shared
    // directory, so a concurrent read always observes a directory that contains
    // every process used by this file.
    unsafe { std::env::set_var("PROCESSES_DIR", test_processes_dir()) };
}

/// Assert that an attempted `post_invoice` advance was refused, that the
/// instance is still in `draft`, that no step log row was written for the
/// capability, and that no GL posting happened.
async fn assert_refused_with_state_unchanged(pool: &PgPool, instance_id: i64, capability: &str) {
    let after = get_instance(pool, instance_id)
        .await
        .expect("get instance after refusal");
    assert_eq!(
        after.instance.current_state, "draft",
        "instance must remain in draft after refusal"
    );
    assert_eq!(
        after.instance.status, "active",
        "instance status must remain active after refusal"
    );
    assert_eq!(
        after.steps.len(),
        1,
        "step log must contain only the start row after refusal"
    );
    assert!(
        after.steps.iter().all(|s| s.capability != capability),
        "no {capability} step log row must be written when the transition is refused"
    );

    let ar = account_balance(pool, "1100").await.expect("AR balance");
    assert_eq!(
        ar.debits, 0,
        "AR must not be debited when transition is refused"
    );
    let sales = account_balance(pool, "4000").await.expect("sales balance");
    assert_eq!(
        sales.credits, 0,
        "sales_revenue must not be credited when transition is refused"
    );
    let tax = account_balance(pool, "2100").await.expect("tax balance");
    assert_eq!(
        tax.credits, 0,
        "tax_payable must not be credited when transition is refused"
    );
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
    set_processes_dir();

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

/// ABSENT-FIELD FAIL-CLOSED: a `customer_invoice` instance whose context has
/// no `lines` key at all must be refused by `lines_nonempty`, the state left
/// unchanged and no ledger entry posted.
#[sqlx::test(migrations = "../../migrations")]
async fn absent_lines_key_is_refused_fail_closed(pool: PgPool) {
    set_processes_dir();

    let context = serde_json::json!({
        "customer": "Acme Corp",
        "total": 60_000
    });

    let instance = start_instance(
        &pool,
        "customer_invoice",
        Some("INV-002".to_string()),
        context,
    )
    .await
    .expect("start instance without lines key");
    let id = instance.id;

    let result = advance_instance(
        &pool,
        id,
        "post_invoice",
        "test_actor",
        AdvanceInput {
            amounts: sample_posting_amounts(),
            context_patch: serde_json::json!({}),
            entry_date: today(),
        },
    )
    .await;

    assert_eq!(
        result.unwrap_err().to_string(),
        "no transition from 'draft' with capability 'post_invoice'. Available from 'draft': guard 'lines_nonempty' refused: context.lines is missing",
        "missing 'lines' must be refused by lines_nonempty (fail-closed)"
    );

    assert_refused_with_state_unchanged(&pool, id, "post_invoice").await;
}

/// ABSENT-FIELD FAIL-CLOSED: a `customer_invoice` instance whose context has
/// no `total` key at all must be refused by `totals_balance`, the state left
/// unchanged and no ledger entry posted.
#[sqlx::test(migrations = "../../migrations")]
async fn absent_total_key_is_refused_fail_closed(pool: PgPool) {
    set_processes_dir();

    let context = serde_json::json!({
        "customer": "Acme Corp",
        "lines": [{ "description": "Widget", "amount": 60_000 }]
    });

    let instance = start_instance(
        &pool,
        "customer_invoice",
        Some("INV-003".to_string()),
        context,
    )
    .await
    .expect("start instance without total key");
    let id = instance.id;

    let result = advance_instance(
        &pool,
        id,
        "post_invoice",
        "test_actor",
        AdvanceInput {
            amounts: sample_posting_amounts(),
            context_patch: serde_json::json!({}),
            entry_date: today(),
        },
    )
    .await;

    assert_eq!(
        result.unwrap_err().to_string(),
        "no transition from 'draft' with capability 'post_invoice'. Available from 'draft': guard 'totals_balance' refused: context.total is missing",
        "missing 'total' must be refused by totals_balance (fail-closed)"
    );

    assert_refused_with_state_unchanged(&pool, id, "post_invoice").await;
}

/// lines_nonempty IN ISOLATION: a context whose `lines` is an empty array must
/// be refused, with the refusal attributable to `lines_nonempty` rather than
/// to `totals_balance`.
#[sqlx::test(migrations = "../../migrations")]
async fn empty_lines_is_refused_by_lines_nonempty(pool: PgPool) {
    set_processes_dir();

    let context = serde_json::json!({
        "customer": "Acme Corp",
        "lines": [],
        "total": 0
    });

    let instance = start_instance(
        &pool,
        "customer_invoice",
        Some("INV-004".to_string()),
        context,
    )
    .await
    .expect("start instance with empty lines");
    let id = instance.id;

    let result = advance_instance(
        &pool,
        id,
        "post_invoice",
        "test_actor",
        AdvanceInput {
            amounts: sample_posting_amounts(),
            context_patch: serde_json::json!({}),
            entry_date: today(),
        },
    )
    .await;

    assert_eq!(
        result.unwrap_err().to_string(),
        "no transition from 'draft' with capability 'post_invoice'. Available from 'draft': guard 'lines_nonempty' refused: context.lines is empty",
        "empty 'lines' must be refused by lines_nonempty"
    );

    assert_refused_with_state_unchanged(&pool, id, "post_invoice").await;
}

/// UNKNOWN GUARD FAILS CLOSED: a transition declaring a guard name the engine
/// does not implement is refused rather than allowed.
#[sqlx::test(migrations = "../../migrations")]
async fn unknown_guard_is_refused_fail_closed(pool: PgPool) {
    set_processes_dir();

    let instance = start_instance(
        &pool,
        "unknown_guard_invoice",
        Some("UG-001".to_string()),
        serde_json::json!({}),
    )
    .await
    .expect("start unknown_guard_invoice instance");
    let id = instance.id;
    assert_eq!(instance.current_state, "draft");

    let result = advance_instance(
        &pool,
        id,
        "post_invoice",
        "test_actor",
        AdvanceInput {
            amounts: sample_posting_amounts(),
            context_patch: serde_json::json!({}),
            entry_date: today(),
        },
    )
    .await;

    assert_eq!(
        result.unwrap_err().to_string(),
        "no transition from 'draft' with capability 'post_invoice'. Available from 'draft': guard 'this_guard_is_not_implemented' refused: unknown guard",
        "unknown guard name must be refused (fail-closed)"
    );

    let after = get_instance(&pool, id)
        .await
        .expect("get instance after refusal");
    assert_eq!(after.instance.current_state, "draft");
    assert_eq!(after.instance.status, "active");
    assert_eq!(after.steps.len(), 1, "only the start row");
    assert!(after.steps.iter().all(|s| s.capability != "post_invoice"));
}
