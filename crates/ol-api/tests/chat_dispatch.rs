//! Integration tests for the deterministic `dispatch_tool` executor in `chat.rs`.
//!
//! Each test gets a fresh migrated database via `#[sqlx::test]`.
//! Migration `0002_seed_chart_of_accounts.sql` populates accounts 1000/4000 and
//! the `GEN` journal.
//!
//! NOTE: The Ollama tool loop in `chat::chat` is NOT tested here — it is
//! non-deterministic and requires a running Ollama daemon with the configured
//! model. Smoke-test it manually via:
//!   curl -s -X POST http://localhost:3000/chat \
//!     -H 'Content-Type: application/json' \
//!     -d '{"message":"What is the balance of account 1000?"}'

use ol_api::chat::{ToolContext, dispatch_tool};
use serde_json::json;
use sqlx::PgPool;

/// A fresh tool context per call: these tests exercise dispatch, not replay.
/// Tests that DO exercise replay build their own stable `ToolContext`.
fn test_ctx() -> ToolContext {
    ToolContext {
        conversation_id: uuid::Uuid::new_v4(),
        turn: 0,
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Post a balanced EUR entry directly through dispatch_tool.
async fn post_balanced_eur(pool: &PgPool, amount_cents: i64) -> serde_json::Value {
    dispatch_tool(
        pool,
        "post_journal_entry",
        &json!({
            "journal_code": "GEN",
            "entry_date": "2025-06-13",
            "memo": "test entry",
            "lines": [
                { "account_code": "1000", "debit": amount_cents, "credit": 0, "currency": "EUR" },
                { "account_code": "4000", "debit": 0, "credit": amount_cents, "currency": "EUR" }
            ]
        }),
        &test_ctx(),
    )
    .await
    .expect("balanced post must succeed")
}

// ─── Tests ────────────────────────────────────────────────────────────────────

/// get_account_balance after a post returns the right balance JSON including currency.
#[sqlx::test(migrations = "../../migrations")]
async fn get_balance_after_post(pool: PgPool) {
    // Post 220_00 EUR (= €220.00) to account 1000.
    post_balanced_eur(&pool, 22_000).await;

    let result = dispatch_tool(
        &pool,
        "get_account_balance",
        &json!({ "account_code": "1000" }),
        &test_ctx(),
    )
    .await
    .expect("get_account_balance must succeed");

    assert_eq!(result["account_code"], "1000", "account_code field");
    assert_eq!(result["currency"], "EUR", "currency must be EUR");
    // 1000 is an asset; debit-positive => balance == debits.
    assert_eq!(result["debits"], 22_000, "debits must equal posted amount");
    assert_eq!(
        result["balance"], 22_000,
        "balance must equal posted amount for asset"
    );
}

/// post_journal_entry with a balanced EUR pair returns correct result JSON and the
/// entry is persisted (balance query confirms it).
#[sqlx::test(migrations = "../../migrations")]
async fn post_balanced_entry_returns_result(pool: PgPool) {
    let result = post_balanced_eur(&pool, 5_000).await;

    assert!(result["entry_id"].is_i64(), "entry_id must be an integer");
    assert_eq!(result["balanced"], true, "balanced must be true");
    assert_eq!(
        result["replayed"], false,
        "replayed must be false on first post"
    );

    // Confirm the entry is actually stored by querying the balance.
    let bal = dispatch_tool(
        &pool,
        "get_account_balance",
        &json!({ "account_code": "1000" }),
        &test_ctx(),
    )
    .await
    .expect("balance query must succeed");
    assert_eq!(bal["debits"], 5_000, "debits must reflect posted entry");
}

/// post_journal_entry with an UNBALANCED pair returns Err containing "UNBALANCED"
/// and writes no entry to the ledger.
#[sqlx::test(migrations = "../../migrations")]
async fn post_unbalanced_entry_returns_err(pool: PgPool) {
    let err = dispatch_tool(
        &pool,
        "post_journal_entry",
        &json!({
            "journal_code": "GEN",
            "entry_date": "2025-06-13",
            "lines": [
                { "account_code": "1000", "debit": 1000, "credit": 0 },
                { "account_code": "4000", "debit": 0,    "credit": 999 }
            ]
        }),
        &test_ctx(),
    )
    .await
    .expect_err("unbalanced post must return Err");

    assert!(
        err.to_uppercase().contains("UNBALANCED"),
        "error message must contain UNBALANCED, got: {err}"
    );

    // Confirm no entry was written — balance should still be zero.
    let bal = dispatch_tool(
        &pool,
        "get_account_balance",
        &json!({ "account_code": "1000" }),
        &test_ctx(),
    )
    .await
    .expect("balance query must succeed");
    assert_eq!(bal["debits"], 0, "no entry must have been written");
}

/// Unknown tool name returns Err.
#[sqlx::test(migrations = "../../migrations")]
async fn unknown_tool_returns_err(pool: PgPool) {
    let err = dispatch_tool(&pool, "drop_all_tables", &json!({}), &test_ctx())
        .await
        .expect_err("unknown tool must return Err");

    assert!(
        err.contains("unknown tool"),
        "error must mention unknown tool, got: {err}"
    );
}

// ─── Engine tool tests ────────────────────────────────────────────────────────

/// GIVEN a valid process name
/// WHEN dispatch_tool("start_process", ..., &test_ctx()) is called
/// THEN an instance is returned at the initial state
#[sqlx::test(migrations = "../../migrations")]
async fn start_process_tool_creates_instance(pool: PgPool) {
    let result = dispatch_tool(
        &pool,
        "start_process",
        &json!({ "process": "order_to_cash", "reference": "SO-CHAT-001" }),
        &test_ctx(),
    )
    .await
    .expect("start_process must succeed");

    assert!(result["id"].is_i64(), "id must be integer");
    assert_eq!(result["process"], "order_to_cash");
    assert_eq!(result["current_state"], "inquiry");
    assert_eq!(result["status"], "active");
    assert_eq!(result["reference"], "SO-CHAT-001");
}

/// GIVEN an instance started via start_process
/// WHEN dispatch_tool("advance_process", ..., &test_ctx()) is called with a valid non-posting capability
/// THEN the instance moves to the next state
#[sqlx::test(migrations = "../../migrations")]
async fn advance_process_tool_non_posting_step(pool: PgPool) {
    // Start instance.
    let start = dispatch_tool(
        &pool,
        "start_process",
        &json!({ "process": "order_to_cash" }),
        &test_ctx(),
    )
    .await
    .expect("start_process must succeed");
    let id = start["id"].as_i64().expect("id must be integer");

    // Advance: inquiry → so_open.
    let result = dispatch_tool(
        &pool,
        "advance_process",
        &json!({ "instance_id": id, "capability": "confirm_order" }),
        &test_ctx(),
    )
    .await
    .expect("advance_process must succeed");

    assert_eq!(result["id"], id);
    assert_eq!(result["current_state"], "so_open");
    assert_eq!(result["status"], "active");
}

/// GIVEN an instance walked to "fulfillment"
/// WHEN dispatch_tool("advance_process", "deliver", amounts {amount: 3000}, &test_ctx())
/// THEN the instance moves to "shipped" and the COGS balance increases
#[sqlx::test(migrations = "../../migrations")]
async fn advance_process_tool_posting_step_moves_balance(pool: PgPool) {
    let id = chat_walk_to_fulfillment(&pool).await;

    // Check COGS before.
    let cogs_before = dispatch_tool(
        &pool,
        "get_account_balance",
        &json!({ "account_code": "5000" }),
        &test_ctx(),
    )
    .await
    .expect("balance query must succeed");
    let debits_before = cogs_before["debits"].as_i64().unwrap_or(0);

    // deliver: posts COGS debit / Inventory credit.
    let result = dispatch_tool(
        &pool,
        "advance_process",
        &json!({
            "instance_id": id,
            "capability": "deliver",
            "amounts": { "amount": 3000 }
        }),
        &test_ctx(),
    )
    .await
    .expect("advance_process must succeed");

    assert_eq!(result["current_state"], "shipped");

    // COGS (account 5000) must have increased.
    let cogs_after = dispatch_tool(
        &pool,
        "get_account_balance",
        &json!({ "account_code": "5000" }),
        &test_ctx(),
    )
    .await
    .expect("balance query must succeed");
    let debits_after = cogs_after["debits"].as_i64().expect("debits must be i64");
    assert_eq!(
        debits_after,
        debits_before + 3000,
        "COGS debits must increase by the posted amount"
    );
}

/// GIVEN an illegal capability for the current state
/// WHEN dispatch_tool("advance_process", ..., &test_ctx()) is called
/// THEN Err is returned so the model can read and explain it
#[sqlx::test(migrations = "../../migrations")]
async fn advance_process_tool_illegal_capability_returns_err(pool: PgPool) {
    let start = dispatch_tool(
        &pool,
        "start_process",
        &json!({ "process": "order_to_cash" }),
        &test_ctx(),
    )
    .await
    .expect("start_process must succeed");
    let id = start["id"].as_i64().expect("id must be integer");

    let err = dispatch_tool(
        &pool,
        "advance_process",
        &json!({ "instance_id": id, "capability": "register_payment" }),
        &test_ctx(),
    )
    .await
    .expect_err("illegal capability must return Err");

    assert!(
        !err.is_empty(),
        "error message must not be empty, got: {err}"
    );
}

/// Walk an order_to_cash instance to "fulfillment" via chat dispatch_tool.
async fn chat_walk_to_fulfillment(pool: &sqlx::PgPool) -> i64 {
    let start = dispatch_tool(
        pool,
        "start_process",
        &json!({ "process": "order_to_cash" }),
        &test_ctx(),
    )
    .await
    .expect("start_process");
    let id = start["id"].as_i64().expect("id");

    for cap in &[
        "confirm_order",
        "run_credit_check",
        "release_credit_hold",
        "allocate_inventory",
    ] {
        dispatch_tool(
            pool,
            "advance_process",
            &json!({ "instance_id": id, "capability": cap }),
            &test_ctx(),
        )
        .await
        .unwrap_or_else(|e| panic!("advance_process {cap} failed: {e}"));
    }

    id
}
