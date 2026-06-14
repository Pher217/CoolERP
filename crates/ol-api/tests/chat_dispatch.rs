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

use ol_api::chat::dispatch_tool;
use serde_json::json;
use sqlx::PgPool;

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
    )
    .await
    .expect("balance query must succeed");
    assert_eq!(bal["debits"], 0, "no entry must have been written");
}

/// Unknown tool name returns Err.
#[sqlx::test(migrations = "../../migrations")]
async fn unknown_tool_returns_err(pool: PgPool) {
    let err = dispatch_tool(&pool, "drop_all_tables", &json!({}))
        .await
        .expect_err("unknown tool must return Err");

    assert!(
        err.contains("unknown tool"),
        "error must mention unknown tool, got: {err}"
    );
}
