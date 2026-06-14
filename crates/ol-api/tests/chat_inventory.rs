//! Integration tests for inventory chat tools and the shared REST query.
//!
//! Migration `0006_inventory_seed.sql` populates one location (MAIN) and four
//! items (WIDGET-A, WIDGET-B, GADGET-1, BOLT-10).  Tests exercise the
//! deterministic `dispatch_tool` executor so no Ollama daemon is required.

use ol_api::chat::dispatch_tool;
use serde_json::json;
use sqlx::PgPool;

/// receive_stock creates a move and list_inventory reports the new on-hand qty.
#[sqlx::test(migrations = "../../migrations")]
async fn receive_stock_then_list_inventory_reflects_qty(pool: PgPool) {
    let result = dispatch_tool(
        &pool,
        "receive_stock",
        &json!({
            "sku": "WIDGET-A",
            "location_code": "MAIN",
            "qty": "10",
            "unit_cost": "250"
        }),
    )
    .await
    .expect("receive_stock must succeed");

    assert_eq!(result["sku"], "WIDGET-A", "sku in response");
    assert_eq!(result["qty"], "10", "qty echoed in response");
    assert!(result["move_id"].is_i64(), "move_id must be an integer");

    let list = dispatch_tool(&pool, "list_inventory", &json!({}))
        .await
        .expect("list_inventory must succeed");
    let items = list
        .as_array()
        .expect("list_inventory must return an array");
    let widget_a = items
        .iter()
        .find(|i| i["sku"] == "WIDGET-A")
        .expect("WIDGET-A must be in inventory list");

    assert_eq!(widget_a["name"], "Widget A", "item name");
    assert_eq!(
        widget_a["on_hand"], "10.0000",
        "on_hand must reflect receipt"
    );
}

/// show_view returns the requested module and focus.
#[sqlx::test(migrations = "../../migrations")]
async fn show_view_dispatch_returns_module(pool: PgPool) {
    let result = dispatch_tool(
        &pool,
        "show_view",
        &json!({ "module": "inventory", "focus": "WIDGET-A" }),
    )
    .await
    .expect("show_view must succeed");

    assert_eq!(result["ok"], true, "ok flag");
    assert_eq!(result["module"], "inventory", "module echoed");
    assert_eq!(result["focus"], "WIDGET-A", "focus echoed");
}

/// show_view rejects an unknown module.
#[sqlx::test(migrations = "../../migrations")]
async fn show_view_invalid_module_returns_err(pool: PgPool) {
    let err = dispatch_tool(&pool, "show_view", &json!({ "module": "bad_module" }))
        .await
        .expect_err("invalid module must fail");

    assert!(
        err.contains("invalid module"),
        "error must mention invalid module, got: {err}"
    );
}

/// receive_stock with a missing SKU returns a clear error and writes no move.
#[sqlx::test(migrations = "../../migrations")]
async fn receive_stock_missing_item_returns_err(pool: PgPool) {
    let err = dispatch_tool(
        &pool,
        "receive_stock",
        &json!({ "sku": "NO-SUCH-SKU", "location_code": "MAIN", "qty": "1" }),
    )
    .await
    .expect_err("missing item must fail");

    assert!(
        err.contains("item not found"),
        "error must mention item not found, got: {err}"
    );
}

/// receive_stock with a missing location returns a clear error and writes no move.
#[sqlx::test(migrations = "../../migrations")]
async fn receive_stock_missing_location_returns_err(pool: PgPool) {
    let err = dispatch_tool(
        &pool,
        "receive_stock",
        &json!({ "sku": "WIDGET-A", "location_code": "NO-SUCH-LOC", "qty": "1" }),
    )
    .await
    .expect_err("missing location must fail");

    assert!(
        err.contains("location not found"),
        "error must mention location not found, got: {err}"
    );
}

/// list_inventory includes all seeded items with zero on_hand before any moves.
#[sqlx::test(migrations = "../../migrations")]
async fn list_inventory_seeded_items_start_at_zero(pool: PgPool) {
    let list = dispatch_tool(&pool, "list_inventory", &json!({}))
        .await
        .expect("list_inventory must succeed");
    let items = list
        .as_array()
        .expect("list_inventory must return an array");

    let skus: Vec<_> = items.iter().map(|i| i["sku"].as_str().unwrap()).collect();
    assert_eq!(skus, vec!["BOLT-10", "GADGET-1", "WIDGET-A", "WIDGET-B"]);

    for item in items {
        assert_eq!(
            item["on_hand"], "0.0000",
            "{} should start at zero",
            item["sku"]
        );
    }
}
