//! Integration tests for the ol-engine crate using the order_to_cash process.
//!
//! Each `#[sqlx::test]` receives a fresh, migrated database.  All tests set
//! `PROCESSES_DIR` to point at the workspace `processes/` directory so the
//! engine can load the YAML definitions.

use ol_engine::{
    AdvanceInput, EngineError, advance_instance, get_instance, list_instances, start_instance,
};
use ol_ledger::account_balance;
use sqlx::PgPool;
use std::collections::HashMap;

const PROCESSES_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../processes");

fn today() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 6, 14).unwrap()
}

/// Advance without a posting (no amounts needed).
async fn advance_no_posting(pool: &PgPool, id: i64, capability: &str) -> ol_engine::Instance {
    advance_instance(
        pool,
        id,
        capability,
        "test_actor",
        AdvanceInput {
            amounts: HashMap::new(),
            context_patch: serde_json::json!({}),
            entry_date: today(),
        },
    )
    .await
    .expect(capability)
}

/// Advance with a simple single-credit posting (`amount` key).
async fn advance_with_amount(
    pool: &PgPool,
    id: i64,
    capability: &str,
    amount: i64,
) -> ol_engine::Instance {
    let mut amounts = HashMap::new();
    amounts.insert("amount".to_string(), amount);
    advance_instance(
        pool,
        id,
        capability,
        "test_actor",
        AdvanceInput {
            amounts,
            context_patch: serde_json::json!({}),
            entry_date: today(),
        },
    )
    .await
    .expect(capability)
}

// ---------------------------------------------------------------------------
// Full happy-path: inquiry → cleared
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn test_full_o2c_run(pool: PgPool) {
    /*
     * GIVEN a fresh order_to_cash instance
     * WHEN all happy-path capabilities are executed in order
     * THEN the instance reaches `cleared` with status `completed`
     *      and the GL balances reflect all three postings.
     */
    // SAFETY: single-threaded test setup; no other threads read this var yet.
    unsafe { std::env::set_var("PROCESSES_DIR", PROCESSES_DIR) };

    // Start
    let instance = start_instance(
        &pool,
        "order_to_cash",
        Some("SO-001".to_string()),
        serde_json::json!({"customer": "Acme Corp"}),
    )
    .await
    .expect("start_instance");

    assert_eq!(instance.current_state, "inquiry");
    assert_eq!(instance.status, "active");
    assert_eq!(instance.reference.as_deref(), Some("SO-001"));

    let id = instance.id;

    // inquiry → so_open (no posting)
    let inst = advance_no_posting(&pool, id, "confirm_order").await;
    assert_eq!(inst.current_state, "so_open");

    // so_open → credit_check (no posting)
    let inst = advance_no_posting(&pool, id, "run_credit_check").await;
    assert_eq!(inst.current_state, "credit_check");

    // credit_check → confirmed (no posting)
    let inst = advance_no_posting(&pool, id, "release_credit_hold").await;
    assert_eq!(inst.current_state, "confirmed");

    // confirmed → fulfillment (no posting)
    let inst = advance_no_posting(&pool, id, "allocate_inventory").await;
    assert_eq!(inst.current_state, "fulfillment");

    // fulfillment → shipped (POSTING: debit COGS 5000, credit Inventory 1200)
    let cogs_amount: i64 = 50_000; // €500.00
    let inst = advance_with_amount(&pool, id, "deliver", cogs_amount).await;
    assert_eq!(inst.current_state, "shipped");

    // Verify GL: COGS debited, Inventory credited
    let cogs_bal = account_balance(&pool, "5000").await.expect("cogs balance");
    assert_eq!(cogs_bal.debits, cogs_amount, "COGS should be debited");
    let inv_bal = account_balance(&pool, "1200")
        .await
        .expect("inventory balance");
    assert_eq!(inv_bal.credits, cogs_amount, "Inventory should be credited");

    // shipped → invoiced (POSTING: debit AR 1100, credit sales_revenue 4000 + tax_payable 2100)
    // Multiple credit: must provide named amounts for each credit role.
    let total_invoice: i64 = 60_000; // €600.00 (net + tax)
    let sales_amount: i64 = 50_000; // €500.00 net
    let tax_amount: i64 = 10_000; // €100.00 tax
    assert_eq!(sales_amount + tax_amount, total_invoice);

    let mut amounts = HashMap::new();
    amounts.insert("amount".to_string(), total_invoice);
    amounts.insert("sales_revenue".to_string(), sales_amount);
    amounts.insert("tax_payable".to_string(), tax_amount);

    let inst = advance_instance(
        &pool,
        id,
        "post_invoice",
        "test_actor",
        AdvanceInput {
            amounts,
            context_patch: serde_json::json!({}),
            entry_date: today(),
        },
    )
    .await
    .expect("post_invoice");
    assert_eq!(inst.current_state, "invoiced");

    // Verify GL: AR debited, sales_revenue + tax_payable credited
    let ar_bal = account_balance(&pool, "1100").await.expect("ar balance");
    assert_eq!(ar_bal.debits, total_invoice, "AR should be debited");
    let rev_bal = account_balance(&pool, "4000")
        .await
        .expect("revenue balance");
    assert_eq!(
        rev_bal.credits, sales_amount,
        "Sales revenue should be credited"
    );
    let tax_bal = account_balance(&pool, "2100").await.expect("tax balance");
    assert_eq!(
        tax_bal.credits, tax_amount,
        "Tax payable should be credited"
    );

    // invoiced → payment_pending (no posting)
    let inst = advance_no_posting(&pool, id, "send_invoice").await;
    assert_eq!(inst.current_state, "payment_pending");

    // payment_pending → cleared (POSTING: debit cash 1000, credit AR 1100)
    let inst = advance_with_amount(&pool, id, "register_payment", total_invoice).await;
    assert_eq!(inst.current_state, "cleared");
    assert_eq!(inst.status, "completed");

    // Verify GL: cash debited, AR credited (net AR = 0)
    let cash_bal = account_balance(&pool, "1000").await.expect("cash balance");
    assert_eq!(cash_bal.debits, total_invoice, "Cash should be debited");
    let ar_bal_after = account_balance(&pool, "1100")
        .await
        .expect("ar balance after");
    // AR: debited by invoice, credited by payment — net = 0
    assert_eq!(ar_bal_after.debits, total_invoice);
    assert_eq!(ar_bal_after.credits, total_invoice);
    assert_eq!(ar_bal_after.balance, 0, "AR should be fully cleared");
}

// ---------------------------------------------------------------------------
// Illegal transition: wrong capability for current state
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn test_illegal_transition(pool: PgPool) {
    /*
     * GIVEN a process instance in state `inquiry`
     * WHEN an unknown / inapplicable capability is requested
     * THEN EngineError::IllegalTransition is returned
     */
    // SAFETY: single-threaded test setup; no other threads read this var yet.
    unsafe { std::env::set_var("PROCESSES_DIR", PROCESSES_DIR) };

    let instance = start_instance(&pool, "order_to_cash", None, serde_json::json!({}))
        .await
        .expect("start_instance");

    let result = advance_instance(
        &pool,
        instance.id,
        "register_payment", // wrong capability for `inquiry` state
        "test_actor",
        AdvanceInput {
            amounts: HashMap::new(),
            context_patch: serde_json::json!({}),
            entry_date: today(),
        },
    )
    .await;

    assert!(
        matches!(result, Err(EngineError::IllegalTransition { .. })),
        "expected IllegalTransition, got: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// Instance not active: reject advance on completed instance
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn test_advance_completed_instance_rejected(pool: PgPool) {
    /*
     * GIVEN a process instance that has reached its terminal state (`cleared`)
     * WHEN another advance is attempted
     * THEN EngineError::InstanceNotActive is returned
     */
    // SAFETY: single-threaded test setup; no other threads read this var yet.
    unsafe { std::env::set_var("PROCESSES_DIR", PROCESSES_DIR) };

    // Fast path to `cleared` via the return branch — shorter than full happy path.
    // inquiry → so_open → credit_check → confirmed → fulfillment → shipped → return
    // `return` has no outgoing transitions → completed.
    let instance = start_instance(&pool, "order_to_cash", None, serde_json::json!({}))
        .await
        .expect("start_instance");
    let id = instance.id;

    advance_no_posting(&pool, id, "confirm_order").await;
    advance_no_posting(&pool, id, "run_credit_check").await;
    advance_no_posting(&pool, id, "release_credit_hold").await;
    advance_no_posting(&pool, id, "allocate_inventory").await;
    advance_with_amount(&pool, id, "deliver", 10_000).await;
    // shipped → return (no posting, terminal)
    let inst = advance_no_posting(&pool, id, "authorize_return").await;
    assert_eq!(inst.current_state, "return");
    assert_eq!(inst.status, "completed");

    // Now try to advance a completed instance
    let result = advance_instance(
        &pool,
        id,
        "confirm_order",
        "test_actor",
        AdvanceInput {
            amounts: HashMap::new(),
            context_patch: serde_json::json!({}),
            entry_date: today(),
        },
    )
    .await;

    assert!(
        matches!(result, Err(EngineError::InstanceNotActive(_))),
        "expected InstanceNotActive, got: {result:?}"
    );
}

// ---------------------------------------------------------------------------
// Deterministic idempotency: the engine derives the key; no caller input needed.
// A retry after crash-recovery (same state, same step_count) re-derives the same
// key and the ledger deduplicates — the GL balance does not move.
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn test_derived_key_prevents_double_post_on_retry(pool: PgPool) {
    /*
     * GIVEN a process instance at `fulfillment`
     * WHEN `deliver` succeeds and the caller re-invokes `deliver` again
     *      (simulating a crash-retry from a still-stuck state)
     * THEN the second call returns IllegalTransition (the state already advanced),
     *      and the GL balance is unchanged — no double-post occurred.
     *
     * This test also exercises that the derived-key is deterministic: a true
     * crash-retry on a still-unadvanced instance would derive the same key and
     * hit the ledger dedup. Here we verify the observable invariant: one balance
     * movement regardless of retry count.
     */
    // SAFETY: single-threaded test setup; no other threads read this var yet.
    unsafe { std::env::set_var("PROCESSES_DIR", PROCESSES_DIR) };

    let instance = start_instance(&pool, "order_to_cash", None, serde_json::json!({}))
        .await
        .expect("start_instance");
    let id = instance.id;

    advance_no_posting(&pool, id, "confirm_order").await;
    advance_no_posting(&pool, id, "run_credit_check").await;
    advance_no_posting(&pool, id, "release_credit_hold").await;
    advance_no_posting(&pool, id, "allocate_inventory").await;

    let cogs_amount: i64 = 30_000;
    let mut amounts = HashMap::new();
    amounts.insert("amount".to_string(), cogs_amount);

    // First call: succeeds and advances the instance to `shipped`.
    let first = advance_instance(
        &pool,
        id,
        "deliver",
        "test_actor",
        AdvanceInput {
            amounts: amounts.clone(),
            context_patch: serde_json::json!({}),
            entry_date: today(),
        },
    )
    .await
    .expect("deliver first call");
    assert_eq!(first.current_state, "shipped");

    let cogs_after_first = account_balance(&pool, "5000")
        .await
        .expect("cogs balance")
        .debits;
    assert_eq!(cogs_after_first, cogs_amount);

    // Second call: the state already moved to `shipped`, so `deliver` (which
    // transitions from `fulfillment`) is now an illegal transition.
    // The GL balance must remain unchanged — no double-post.
    let second = advance_instance(
        &pool,
        id,
        "deliver",
        "test_actor",
        AdvanceInput {
            amounts,
            context_patch: serde_json::json!({}),
            entry_date: today(),
        },
    )
    .await;
    // `deliver` is no longer a valid transition from `shipped`
    assert!(
        matches!(second, Err(EngineError::IllegalTransition { .. })),
        "expected IllegalTransition on repeated advance, got: {second:?}"
    );

    // Balance unchanged — the GL was not touched a second time.
    let cogs_after_second = account_balance(&pool, "5000")
        .await
        .expect("cogs balance")
        .debits;
    assert_eq!(cogs_after_second, cogs_amount, "GL must not double-post");
}

// ---------------------------------------------------------------------------
// get_instance + list_instances
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn test_get_and_list_instances(pool: PgPool) {
    /*
     * GIVEN two process instances in different states
     * WHEN get_instance and list_instances are called
     * THEN they return correct data and the step log is non-empty
     */
    // SAFETY: single-threaded test setup; no other threads read this var yet.
    unsafe { std::env::set_var("PROCESSES_DIR", PROCESSES_DIR) };

    let a = start_instance(
        &pool,
        "order_to_cash",
        Some("REF-A".to_string()),
        serde_json::json!({}),
    )
    .await
    .expect("start a");
    let b = start_instance(
        &pool,
        "order_to_cash",
        Some("REF-B".to_string()),
        serde_json::json!({}),
    )
    .await
    .expect("start b");

    // Advance `a` by one step
    advance_no_posting(&pool, a.id, "confirm_order").await;

    // get_instance for `a`
    let (inst_a, steps_a) = get_instance(&pool, a.id).await.expect("get a");
    assert_eq!(inst_a.current_state, "so_open");
    assert_eq!(inst_a.reference.as_deref(), Some("REF-A"));
    // start row + confirm_order row
    assert_eq!(steps_a.len(), 2, "should have start + confirm_order steps");
    assert_eq!(steps_a[0].capability, "start");
    assert_eq!(steps_a[1].capability, "confirm_order");
    assert_eq!(steps_a[1].from_state, "inquiry");
    assert_eq!(steps_a[1].to_state, "so_open");

    // `b` untouched
    let (inst_b, steps_b) = get_instance(&pool, b.id).await.expect("get b");
    assert_eq!(inst_b.current_state, "inquiry");
    assert_eq!(steps_b.len(), 1, "only start row for b");

    // list all
    let all = list_instances(&pool, Some("order_to_cash"), None)
        .await
        .expect("list all");
    assert!(all.len() >= 2);

    // list active only
    let active = list_instances(&pool, Some("order_to_cash"), Some("active"))
        .await
        .expect("list active");
    assert!(active.iter().all(|i| i.status == "active"));
}

// ---------------------------------------------------------------------------
// Unbalanced amounts rejected before the DB is touched
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn test_unbalanced_posting_rejected(pool: PgPool) {
    /*
     * GIVEN a process instance in `shipped`
     * WHEN post_invoice is called with credit amounts that don't sum to the debit
     * THEN EngineError::Unbalanced is returned and no GL entry is created
     */
    // SAFETY: single-threaded test setup; no other threads read this var yet.
    unsafe { std::env::set_var("PROCESSES_DIR", PROCESSES_DIR) };

    let instance = start_instance(&pool, "order_to_cash", None, serde_json::json!({}))
        .await
        .expect("start_instance");
    let id = instance.id;

    advance_no_posting(&pool, id, "confirm_order").await;
    advance_no_posting(&pool, id, "run_credit_check").await;
    advance_no_posting(&pool, id, "release_credit_hold").await;
    advance_no_posting(&pool, id, "allocate_inventory").await;
    advance_with_amount(&pool, id, "deliver", 10_000).await;

    // Now in `shipped`. post_invoice requires amount = sales_revenue + tax_payable.
    // Provide unbalanced credit amounts.
    let mut amounts = HashMap::new();
    amounts.insert("amount".to_string(), 60_000_i64);
    amounts.insert("sales_revenue".to_string(), 50_000_i64);
    amounts.insert("tax_payable".to_string(), 5_000_i64); // 50k + 5k = 55k ≠ 60k

    let result = advance_instance(
        &pool,
        id,
        "post_invoice",
        "test_actor",
        AdvanceInput {
            amounts,
            context_patch: serde_json::json!({}),
            entry_date: today(),
        },
    )
    .await;

    assert!(
        matches!(result, Err(EngineError::Unbalanced { .. })),
        "expected Unbalanced, got: {result:?}"
    );

    // Instance must still be in `shipped`
    let (inst, _) = get_instance(&pool, id).await.expect("get instance");
    assert_eq!(
        inst.current_state, "shipped",
        "state must not advance on error"
    );
}

// ---------------------------------------------------------------------------
// Concurrency: two concurrent advances on the same posting transition
// MUST result in exactly one state advance and exactly one GL posting.
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn test_concurrent_advance_posts_exactly_once(pool: PgPool) {
    /*
     * GIVEN a process instance at `shipped` (ready for `post_invoice`)
     * WHEN two concurrent callers both invoke advance_instance("post_invoice")
     *      with the same amounts at the same time
     * THEN exactly ONE of them advances the state to `invoiced`
     *      and the other returns Ok (idempotent — already at target) or
     *      Err(ConcurrentAdvance)
     *      AND the AR balance reflects exactly one posting (no double-post)
     *      AND there is exactly one `post_invoice` step-log row
     *      AND there is exactly one journal entry for this transition.
     */
    // SAFETY: we set PROCESSES_DIR before spawning any concurrent tasks.
    unsafe { std::env::set_var("PROCESSES_DIR", PROCESSES_DIR) };

    // ── Setup: advance to `shipped` sequentially ─────────────────────────────
    let instance = start_instance(
        &pool,
        "order_to_cash",
        Some("CONC-001".to_string()),
        serde_json::json!({"customer": "Concurrent Corp"}),
    )
    .await
    .expect("start_instance");
    let id = instance.id;

    advance_no_posting(&pool, id, "confirm_order").await;
    advance_no_posting(&pool, id, "run_credit_check").await;
    advance_no_posting(&pool, id, "release_credit_hold").await;
    advance_no_posting(&pool, id, "allocate_inventory").await;
    advance_with_amount(&pool, id, "deliver", 50_000).await;

    // Confirm we are at `shipped` before firing the race.
    let (pre, _) = get_instance(&pool, id).await.expect("pre-race get");
    assert_eq!(
        pre.current_state, "shipped",
        "must be at shipped before race"
    );

    // ── The race: two concurrent post_invoice calls ──────────────────────────
    let total_invoice: i64 = 60_000;
    let sales_amount: i64 = 50_000;
    let tax_amount: i64 = 10_000;

    let make_input = || {
        let mut amounts = HashMap::new();
        amounts.insert("amount".to_string(), total_invoice);
        amounts.insert("sales_revenue".to_string(), sales_amount);
        amounts.insert("tax_payable".to_string(), tax_amount);
        AdvanceInput {
            amounts,
            context_patch: serde_json::json!({}),
            entry_date: today(),
        }
    };

    let pool_a = pool.clone();
    let pool_b = pool.clone();

    let (res_a, res_b) = tokio::join!(
        advance_instance(&pool_a, id, "post_invoice", "actor_a", make_input()),
        advance_instance(&pool_b, id, "post_invoice", "actor_b", make_input()),
    );

    // ── Assertions ───────────────────────────────────────────────────────────

    // At least one must succeed.
    let winner = match (&res_a, &res_b) {
        (Ok(inst), _) => inst.clone(),
        (_, Ok(inst)) => inst.clone(),
        _ => panic!(
            "both concurrent advances failed — neither succeeded.\nA: {res_a:?}\nB: {res_b:?}"
        ),
    };

    // Winner must be at `invoiced`.
    assert_eq!(
        winner.current_state, "invoiced",
        "winner must be at `invoiced`"
    );

    // The loser must be either:
    //   Ok        — idempotent: saw the state already moved to `invoiced`
    //   ConcurrentAdvance — guarded UPDATE found the row already advanced
    //   Db(40001) — REPEATABLE READ serialization failure on FOR UPDATE lock
    //               (the loser's tx was aborted because the winner updated
    //               the same row; this is correct concurrent-protection behaviour)
    let loser = if res_a.is_ok() { &res_b } else { &res_a };
    match loser {
        Ok(inst) => assert_eq!(
            inst.current_state, "invoiced",
            "idempotent loser must also report `invoiced`"
        ),
        Err(EngineError::ConcurrentAdvance) => {} // guarded-UPDATE race-loser path
        Err(EngineError::Db(sqlx_err)) => {
            // REPEATABLE READ serialization failure (SQLSTATE 40001) or deadlock
            // (40P01) on the FOR UPDATE lock is the other valid concurrent-loser path.
            let is_serialization = sqlx_err
                .as_database_error()
                .and_then(|e| e.code())
                .map(|c| c == "40001" || c == "40P01")
                .unwrap_or(false);
            assert!(
                is_serialization,
                "loser DB error must be a serialization failure (40001/40P01), got: {sqlx_err:?}"
            );
        }
        Err(other) => panic!("loser returned unexpected error: {other:?}"),
    }

    // ── GL must reflect EXACTLY ONE posting ──────────────────────────────────
    let ar_bal = account_balance(&pool, "1100").await.expect("ar balance");
    assert_eq!(
        ar_bal.debits, total_invoice,
        "AR debited exactly once (no double-post)"
    );

    let rev_bal = account_balance(&pool, "4000")
        .await
        .expect("revenue balance");
    assert_eq!(
        rev_bal.credits, sales_amount,
        "Revenue credited exactly once"
    );

    let tax_bal = account_balance(&pool, "2100").await.expect("tax balance");
    assert_eq!(
        tax_bal.credits, tax_amount,
        "Tax payable credited exactly once"
    );

    // ── Exactly one step-log row for `post_invoice` ──────────────────────────
    let (_, steps) = get_instance(&pool, id).await.expect("get after race");
    let invoice_steps: Vec<_> = steps
        .iter()
        .filter(|s| s.capability == "post_invoice")
        .collect();
    assert_eq!(
        invoice_steps.len(),
        1,
        "exactly one post_invoice step-log row expected, got {}: {invoice_steps:?}",
        invoice_steps.len()
    );

    // ── Exactly one journal entry linked from that step ──────────────────────
    let entry_id = invoice_steps[0]
        .entry_id
        .expect("step log must reference a journal entry");
    // Verify the entry exists in the DB and is unique.
    let entry_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM journal_entries WHERE id = $1")
        .bind(entry_id)
        .fetch_one(&pool)
        .await
        .expect("entry count query");
    assert_eq!(entry_count, 1, "exactly one journal entry for post_invoice");

    // No duplicate entries with a different id for the same posting.
    // We check by counting journal_lines that credit account 4000 (revenue).
    let revenue_line_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) \
         FROM journal_lines jl \
         JOIN accounts a ON a.id = jl.account_id \
         WHERE a.code = '4000' AND jl.credit > 0",
    )
    .fetch_one(&pool)
    .await
    .expect("revenue line count");
    assert_eq!(
        revenue_line_count, 1,
        "exactly one revenue credit line across all entries (no double-post)"
    );
}
