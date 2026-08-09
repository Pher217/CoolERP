//! Integration tests for the ol-engine crate using the order_to_cash process.
//!
//! Each `#[sqlx::test]` receives a fresh, migrated database.  All tests set
//! `PROCESSES_DIR` to point at the workspace `processes/` directory so the
//! engine can load the YAML definitions.

use ol_engine::{
    AdvanceInput, EngineError, advance_instance, available_transitions, get_instance,
    list_instances, start_instance,
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
    let res_a = get_instance(&pool, a.id).await.expect("get a");
    assert_eq!(res_a.instance.current_state, "so_open");
    assert_eq!(res_a.instance.reference.as_deref(), Some("REF-A"));
    // start row + confirm_order row
    assert_eq!(
        res_a.steps.len(),
        2,
        "should have start + confirm_order steps"
    );
    assert_eq!(res_a.steps[0].capability, "start");
    assert_eq!(res_a.steps[1].capability, "confirm_order");
    assert_eq!(res_a.steps[1].from_state, "inquiry");
    assert_eq!(res_a.steps[1].to_state, "so_open");
    // so_open has one outgoing transition: run_credit_check
    assert_eq!(
        res_a.available.len(),
        1,
        "so_open has 1 available transition"
    );
    assert_eq!(res_a.available[0].capability, "run_credit_check");
    assert_eq!(res_a.available[0].to_state, "credit_check");
    assert!(
        res_a.available[0].posting.is_none(),
        "run_credit_check is non-posting"
    );

    // `b` untouched
    let res_b = get_instance(&pool, b.id).await.expect("get b");
    assert_eq!(res_b.instance.current_state, "inquiry");
    assert_eq!(res_b.steps.len(), 1, "only start row for b");

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
    let res = get_instance(&pool, id).await.expect("get instance");
    assert_eq!(
        res.instance.current_state, "shipped",
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
    let pre = get_instance(&pool, id).await.expect("pre-race get");
    assert_eq!(
        pre.instance.current_state, "shipped",
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

    // The loser must be one of:
    //   Ok        — idempotent: saw the state already moved to `invoiced`
    //   ConcurrentAdvance — guarded UPDATE found the row already advanced
    //   Db(40001) — REPEATABLE READ serialization failure on FOR UPDATE lock
    //               (the loser's tx was aborted because the winner updated
    //               the same row; this is correct concurrent-protection behaviour)
    //   IllegalTransition { from: "invoiced" } — the loser took its FOR UPDATE lock
    //               only after the winner committed, so it read the already-advanced
    //               state and refused `post_invoice` as illegal from `invoiced`.
    //               Omitting this arm made the test flaky; it is a legal outcome of
    //               the race, not a failure of the ledger invariant.
    //
    //               TOLERATED, NOT ENDORSED. This outcome is safe but NOT idempotent:
    //               a caller retrying a dropped response gets an error rather than a
    //               replay of the original success. That is exactly the limitation
    //               tracked in #54 — the server-derived key includes the step count,
    //               so it cannot dedupe a post-commit retry. This test is scoped to
    //               ledger exactly-once safety (no double-post), which the GL
    //               assertions below carry; it deliberately does not assert a retry
    //               contract. Do not read this arm as blessing the current retry
    //               semantics.
    //
    //               NOTE ON COVERAGE: `tokio::join!` starts both futures together but
    //               does not force their transactions to overlap. Accepting this arm
    //               means the test can pass on a run where the loser began only after
    //               the winner committed — i.e. without exercising lock contention at
    //               all. Real contention coverage would need a barrier or a test hook.
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
        Err(EngineError::IllegalTransition {
            from, capability, ..
        }) => {
            assert_eq!(
                from, "invoiced",
                "loser rejected the transition from the post-advance state"
            );
            assert_eq!(
                capability, "post_invoice",
                "loser rejected the capability it actually attempted"
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
    let res = get_instance(&pool, id).await.expect("get after race");
    let invoice_steps: Vec<_> = res
        .steps
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

// ---------------------------------------------------------------------------
// PostingAmountsRequired: missing credit role keys for multi-credit posting
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn test_post_invoice_missing_credit_amounts_returns_posting_amounts_required(pool: PgPool) {
    /*
     * GIVEN a process instance at `shipped` (ready for `post_invoice`)
     * WHEN advance_instance("post_invoice") is called with amounts={"amount":71400}
     *      (debit total only, credit role keys sales_revenue and tax_payable absent)
     * THEN Err(PostingAmountsRequired{..}) is returned
     *      AND the error message names both missing roles
     *      AND the instance remains in `shipped` (no state advance)
     */
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

    // In `shipped`. Call post_invoice with only the debit total — credit role keys absent.
    let mut amounts = HashMap::new();
    amounts.insert("amount".to_string(), 71_400_i64);

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

    match &result {
        Err(EngineError::PostingAmountsRequired {
            capability,
            missing,
            credit_roles,
            ..
        }) => {
            assert_eq!(capability, "post_invoice");
            assert!(
                missing.contains(&"sales_revenue".to_string()),
                "missing must include 'sales_revenue', got: {missing:?}"
            );
            assert!(
                missing.contains(&"tax_payable".to_string()),
                "missing must include 'tax_payable', got: {missing:?}"
            );
            // The error message must name both credit roles.
            let msg = result.as_ref().unwrap_err().to_string();
            assert!(
                msg.contains("sales_revenue"),
                "error message must name 'sales_revenue': {msg}"
            );
            assert!(
                msg.contains("tax_payable"),
                "error message must name 'tax_payable': {msg}"
            );
            // credit_roles should list both expected roles.
            assert!(credit_roles.contains(&"sales_revenue".to_string()));
            assert!(credit_roles.contains(&"tax_payable".to_string()));
        }
        other => panic!("expected PostingAmountsRequired, got: {other:?}"),
    }

    // Instance must not have advanced.
    let res = get_instance(&pool, id).await.expect("get instance");
    assert_eq!(
        res.instance.current_state, "shipped",
        "state must not advance when amounts are missing"
    );
}

// ---------------------------------------------------------------------------
// PostingAmountsRequired success path: correct multi-credit amounts post cleanly
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn test_post_invoice_with_correct_multi_credit_amounts_posts(pool: PgPool) {
    /*
     * GIVEN a process instance at `shipped`
     * WHEN advance_instance("post_invoice") is called with
     *      amounts={"amount":71400, "sales_revenue":60000, "tax_payable":11400}
     * THEN the instance advances to `invoiced`
     *      AND AR is debited 71400, sales_revenue credited 60000, tax_payable credited 11400
     */
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

    // Full correct multi-credit amounts.
    let mut amounts = HashMap::new();
    amounts.insert("amount".to_string(), 71_400_i64);
    amounts.insert("sales_revenue".to_string(), 60_000_i64);
    amounts.insert("tax_payable".to_string(), 11_400_i64);

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
    .expect("post_invoice with correct amounts");

    assert_eq!(inst.current_state, "invoiced");

    // Verify GL postings.
    let ar_bal = account_balance(&pool, "1100").await.expect("ar balance");
    assert_eq!(ar_bal.debits, 71_400, "AR debited 71400");

    let rev_bal = account_balance(&pool, "4000")
        .await
        .expect("revenue balance");
    assert_eq!(rev_bal.credits, 60_000, "sales_revenue credited 60000");

    let tax_bal = account_balance(&pool, "2100").await.expect("tax balance");
    assert_eq!(tax_bal.credits, 11_400, "tax_payable credited 11400");
}

// ---------------------------------------------------------------------------
// available_transitions: pure unit tests (no DB needed)
// ---------------------------------------------------------------------------

/// GIVEN the order_to_cash process definition
/// WHEN available_transitions is called for state "so_open"
/// THEN it returns exactly one transition: run_credit_check -> credit_check (non-posting)
#[test]
fn test_available_transitions_so_open() {
    /*
     * GIVEN the order_to_cash process
     * WHEN available_transitions("so_open") is called
     * THEN run_credit_check -> credit_check is the only result, and it has no posting
     */
    use ol_process::Process;
    use std::path::Path;

    let yaml = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../processes/order_to_cash.yaml"),
    )
    .expect("order_to_cash.yaml must be readable");
    let proc = Process::from_yaml(&yaml).expect("must parse");

    let avail = available_transitions(&proc, "so_open");

    assert_eq!(avail.len(), 1, "so_open has exactly 1 outgoing transition");
    assert_eq!(avail[0].capability, "run_credit_check");
    assert_eq!(avail[0].to_state, "credit_check");
    assert!(
        avail[0].posting.is_none(),
        "run_credit_check is a non-posting transition"
    );
}

/// GIVEN the order_to_cash process definition
/// WHEN available_transitions is called for state "invoiced"
/// THEN it returns send_invoice -> payment_pending and issue_credit_memo -> return
#[test]
fn test_available_transitions_invoiced_has_send_invoice_and_credit_memo() {
    /*
     * GIVEN the order_to_cash process
     * WHEN available_transitions("invoiced") is called
     * THEN send_invoice -> payment_pending (non-posting) and
     *      issue_credit_memo -> return (non-posting) are returned
     */
    use ol_process::Process;
    use std::path::Path;

    let yaml = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../processes/order_to_cash.yaml"),
    )
    .expect("order_to_cash.yaml must be readable");
    let proc = Process::from_yaml(&yaml).expect("must parse");

    let avail = available_transitions(&proc, "invoiced");

    let caps: Vec<&str> = avail.iter().map(|a| a.capability.as_str()).collect();
    assert!(
        caps.contains(&"send_invoice"),
        "must include send_invoice, got: {caps:?}"
    );
    assert!(
        caps.contains(&"issue_credit_memo"),
        "must include issue_credit_memo, got: {caps:?}"
    );

    let send = avail
        .iter()
        .find(|a| a.capability == "send_invoice")
        .unwrap();
    assert_eq!(send.to_state, "payment_pending");
    assert!(send.posting.is_none(), "send_invoice is non-posting");

    let memo = avail
        .iter()
        .find(|a| a.capability == "issue_credit_memo")
        .unwrap();
    assert_eq!(memo.to_state, "return");
}

/// GIVEN the order_to_cash process definition
/// WHEN available_transitions is called for a terminal state "cleared"
/// THEN the result is empty
#[test]
fn test_available_transitions_terminal_state_is_empty() {
    /*
     * GIVEN the order_to_cash process
     * WHEN available_transitions("cleared") is called
     * THEN the result is empty (terminal state)
     */
    use ol_process::Process;
    use std::path::Path;

    let yaml = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../processes/order_to_cash.yaml"),
    )
    .expect("order_to_cash.yaml must be readable");
    let proc = Process::from_yaml(&yaml).expect("must parse");

    let avail = available_transitions(&proc, "cleared");
    assert!(
        avail.is_empty(),
        "cleared is terminal — no available transitions"
    );
}

/// GIVEN the order_to_cash process definition
/// WHEN available_transitions is called for "fulfillment"
/// THEN the result includes deliver -> shipped with a posting requirement
#[test]
fn test_available_transitions_posting_transition_exposes_posting_requirement() {
    /*
     * GIVEN the order_to_cash process
     * WHEN available_transitions("fulfillment") is called
     * THEN deliver -> shipped is present with posting.debit_role = "cost_of_goods_sold"
     */
    use ol_process::Process;
    use std::path::Path;

    let yaml = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../processes/order_to_cash.yaml"),
    )
    .expect("order_to_cash.yaml must be readable");
    let proc = Process::from_yaml(&yaml).expect("must parse");

    let avail = available_transitions(&proc, "fulfillment");

    assert_eq!(avail.len(), 1, "fulfillment has exactly 1 transition");
    let deliver = &avail[0];
    assert_eq!(deliver.capability, "deliver");
    assert_eq!(deliver.to_state, "shipped");

    let posting = deliver
        .posting
        .as_ref()
        .expect("deliver must have a posting requirement");
    assert_eq!(posting.debit_role, "cost_of_goods_sold");
    // Single credit: credit_roles is empty (only "amount" key needed)
    assert!(
        posting.credit_roles.is_empty(),
        "single-credit posting has no extra credit roles, got: {:?}",
        posting.credit_roles
    );
}

// ---------------------------------------------------------------------------
// IllegalTransition: enriched error message lists available capabilities
// ---------------------------------------------------------------------------

#[sqlx::test(migrations = "../../migrations")]
async fn test_illegal_transition_error_lists_available_capabilities(pool: PgPool) {
    /*
     * GIVEN a process instance at state "so_open"
     * WHEN an illegal capability is attempted (e.g. "credit_check" — a state, not a capability)
     * THEN IllegalTransition is returned and its Display message names
     *      the real capability: "run_credit_check -> credit_check"
     */
    unsafe { std::env::set_var("PROCESSES_DIR", PROCESSES_DIR) };

    let instance = start_instance(&pool, "order_to_cash", None, serde_json::json!({}))
        .await
        .expect("start_instance");
    let id = instance.id;

    // Advance to so_open
    advance_no_posting(&pool, id, "confirm_order").await;

    // Try "credit_check" (a state name, not a capability) — classic AI mistake
    let result = advance_instance(
        &pool,
        id,
        "credit_check",
        "test_actor",
        AdvanceInput {
            amounts: HashMap::new(),
            context_patch: serde_json::json!({}),
            entry_date: today(),
        },
    )
    .await;

    match &result {
        Err(EngineError::IllegalTransition {
            from,
            capability,
            available,
        }) => {
            assert_eq!(from, "so_open");
            assert_eq!(capability, "credit_check");
            // The available list must contain the real capability name
            assert!(
                available.iter().any(|s| s.contains("run_credit_check")),
                "available list must contain 'run_credit_check', got: {available:?}"
            );
            // The Display message must contain the hint
            let msg = result.as_ref().unwrap_err().to_string();
            assert!(
                msg.contains("run_credit_check"),
                "error message must name 'run_credit_check': {msg}"
            );
            assert!(
                msg.contains("so_open"),
                "error message must name the current state: {msg}"
            );
        }
        other => panic!("expected IllegalTransition, got: {other:?}"),
    }
}
