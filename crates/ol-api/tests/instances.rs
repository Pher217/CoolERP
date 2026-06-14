//! Integration tests for the process-instance REST endpoints.
//!
//! Each test gets a fresh migrated database via `#[sqlx::test]`.
//! Migration `0007_process_engine.sql` creates `process_instances`,
//! `process_steps_log`, `account_roles`, and the `instance_status` enum.
//!
//! The `order_to_cash` process is used throughout: it has both non-posting
//! transitions (e.g. `confirm_order`) and posting transitions (e.g. `deliver`
//! which posts COGS→Inventory).
//!
//! PROCESSES_DIR is set by the test harness (CARGO_MANIFEST_DIR-relative) so
//! the engine can load YAML without a running binary.

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use ol_api::app;
use ol_sdk::{ErrorCode, ErrorEnvelope};
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

// ─── POST /instances ──────────────────────────────────────────────────────────

/// GIVEN a valid process name
/// WHEN POST /instances is called
/// THEN a new instance is created at the initial state with status "active"
#[sqlx::test(migrations = "../../migrations")]
async fn start_instance_creates_active_instance(pool: PgPool) {
    let (status, body) = post_json(pool, "/instances", json!({ "process": "order_to_cash" })).await;

    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert!(body["id"].is_i64(), "id must be integer");
    assert_eq!(body["process"], "order_to_cash");
    // inquiry is the first state (no incoming transitions)
    assert_eq!(body["current_state"], "inquiry");
    assert_eq!(body["status"], "active");
}

/// GIVEN an unknown process name
/// WHEN POST /instances is called
/// THEN 404 with an ErrorEnvelope
#[sqlx::test(migrations = "../../migrations")]
async fn start_instance_unknown_process_returns_404(pool: PgPool) {
    let (status, body) =
        post_json(pool, "/instances", json!({ "process": "does_not_exist" })).await;

    assert_eq!(status, StatusCode::NOT_FOUND, "body={body}");
    let env: ErrorEnvelope = serde_json::from_value(body).expect("must be ErrorEnvelope");
    assert_eq!(env.error.code, ErrorCode::Validation);
}

// ─── POST /instances/{id}/advance — non-posting step ─────────────────────────

/// GIVEN an active order_to_cash instance at "inquiry"
/// WHEN advance with capability "confirm_order"
/// THEN instance moves to "so_open" (non-posting, no GL entry)
#[sqlx::test(migrations = "../../migrations")]
async fn advance_non_posting_step_moves_state(pool: PgPool) {
    // Start instance.
    let (start_status, start_body) = post_json(
        pool.clone(),
        "/instances",
        json!({ "process": "order_to_cash", "reference": "SO-001" }),
    )
    .await;
    assert_eq!(start_status, StatusCode::CREATED, "start body={start_body}");
    let id = start_body["id"].as_i64().expect("id must be integer");

    // Advance: inquiry → so_open.
    let (adv_status, adv_body) = post_json(
        pool,
        &format!("/instances/{id}/advance"),
        json!({ "capability": "confirm_order" }),
    )
    .await;

    assert_eq!(adv_status, StatusCode::OK, "advance body={adv_body}");
    assert_eq!(adv_body["id"], id);
    assert_eq!(adv_body["current_state"], "so_open");
    assert_eq!(adv_body["status"], "active");
}

// ─── POST /instances/{id}/advance — posting step ─────────────────────────────

/// GIVEN an order_to_cash instance walked through to "fulfillment"
/// WHEN advance with capability "deliver" and amounts {amount: 5000}
/// THEN instance moves to "shipped", a GL entry is posted, and the COGS balance moves.
///
/// The "deliver" transition posts:
///   debit:  cost_of_goods_sold → account 5000 (COGS expense)
///   credit: inventory          → account 1200 (Inventory asset)
#[sqlx::test(migrations = "../../migrations")]
async fn advance_posting_step_moves_balance(pool: PgPool) {
    let id = walk_to_fulfillment(pool.clone()).await;

    // Check initial balances.
    let (_, cogs_before) = get(pool.clone(), "/accounts/5000/balance").await;
    let cogs_before_debits = cogs_before["debits"].as_i64().unwrap_or(0);

    // Advance: fulfillment → shipped (posts COGS / Inventory).
    let (adv_status, adv_body) = post_json(
        pool.clone(),
        &format!("/instances/{id}/advance"),
        json!({
            "capability": "deliver",
            "amounts": { "amount": 5000 },
            "entry_date": "2026-06-01"
        }),
    )
    .await;

    assert_eq!(adv_status, StatusCode::OK, "advance body={adv_body}");
    assert_eq!(adv_body["current_state"], "shipped");
    assert_eq!(adv_body["status"], "active");

    // The COGS account (5000) should have been debited by 5000 cents.
    let (_, cogs_after) = get(pool, "/accounts/5000/balance").await;
    let cogs_after_debits = cogs_after["debits"].as_i64().expect("debits must be i64");
    assert_eq!(
        cogs_after_debits,
        cogs_before_debits + 5000,
        "COGS debits must increase by the posted amount"
    );
}

// ─── GET /instances ───────────────────────────────────────────────────────────

/// GIVEN a started instance
/// WHEN GET /instances (no filter)
/// THEN the list includes the new instance
#[sqlx::test(migrations = "../../migrations")]
async fn list_instances_returns_started_instance(pool: PgPool) {
    let (start_status, start_body) = post_json(
        pool.clone(),
        "/instances",
        json!({ "process": "order_to_cash" }),
    )
    .await;
    assert_eq!(start_status, StatusCode::CREATED, "start body={start_body}");
    let id = start_body["id"].as_i64().expect("id must be integer");

    let (status, body) = get(pool, "/instances").await;
    assert_eq!(status, StatusCode::OK, "list body={body}");

    let list = body.as_array().expect("must be an array");
    assert!(
        list.iter().any(|inst| inst["id"] == id),
        "instance {id} must appear in list"
    );
}

/// GIVEN started instances for two processes
/// WHEN GET /instances?process=order_to_cash
/// THEN only order_to_cash instances are returned
#[sqlx::test(migrations = "../../migrations")]
async fn list_instances_process_filter(pool: PgPool) {
    // Start order_to_cash.
    let (s1, b1) = post_json(
        pool.clone(),
        "/instances",
        json!({ "process": "order_to_cash" }),
    )
    .await;
    assert_eq!(s1, StatusCode::CREATED, "b1={b1}");
    let o2c_id = b1["id"].as_i64().unwrap();

    // Start customer_invoice.
    let (s2, b2) = post_json(
        pool.clone(),
        "/instances",
        json!({ "process": "customer_invoice" }),
    )
    .await;
    assert_eq!(s2, StatusCode::CREATED, "b2={b2}");

    let (status, body) = get(pool, "/instances?process=order_to_cash").await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let list = body.as_array().expect("must be array");
    assert!(
        list.iter().all(|inst| inst["process"] == "order_to_cash"),
        "all returned instances must be order_to_cash"
    );
    assert!(
        list.iter().any(|inst| inst["id"] == o2c_id),
        "must include the started order_to_cash instance"
    );
}

// ─── GET /instances/{id} ─────────────────────────────────────────────────────

/// GIVEN a started instance
/// WHEN GET /instances/{id}
/// THEN the response contains the instance and a step log with the "start" entry
#[sqlx::test(migrations = "../../migrations")]
async fn get_instance_returns_instance_and_steps(pool: PgPool) {
    let (start_status, start_body) = post_json(
        pool.clone(),
        "/instances",
        json!({ "process": "order_to_cash", "reference": "REF-42" }),
    )
    .await;
    assert_eq!(start_status, StatusCode::CREATED, "start body={start_body}");
    let id = start_body["id"].as_i64().expect("id must be integer");

    let (status, body) = get(pool, &format!("/instances/{id}")).await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    assert_eq!(body["instance"]["id"], id);
    assert_eq!(body["instance"]["process"], "order_to_cash");
    assert_eq!(body["instance"]["reference"], "REF-42");

    let steps = body["steps"].as_array().expect("steps must be array");
    assert!(!steps.is_empty(), "step log must not be empty after start");
    assert_eq!(steps[0]["capability"], "start");
    assert_eq!(steps[0]["to_state"], "inquiry");
}

/// GIVEN an unknown instance id
/// WHEN GET /instances/{id}
/// THEN 404 with ErrorEnvelope
#[sqlx::test(migrations = "../../migrations")]
async fn get_instance_not_found_returns_404(pool: PgPool) {
    let (status, body) = get(pool, "/instances/99999").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body={body}");
    let env: ErrorEnvelope = serde_json::from_value(body).expect("must be ErrorEnvelope");
    assert_eq!(env.error.code, ErrorCode::Validation);
}

// ─── Illegal capability → 409 ─────────────────────────────────────────────────

/// GIVEN an instance at "inquiry"
/// WHEN advance with an illegal capability (not a valid transition from inquiry)
/// THEN 409 with ErrorEnvelope
#[sqlx::test(migrations = "../../migrations")]
async fn advance_illegal_capability_returns_409(pool: PgPool) {
    let (start_status, start_body) = post_json(
        pool.clone(),
        "/instances",
        json!({ "process": "order_to_cash" }),
    )
    .await;
    assert_eq!(start_status, StatusCode::CREATED, "start body={start_body}");
    let id = start_body["id"].as_i64().expect("id must be integer");

    // "register_payment" is only valid from "payment_pending", not "inquiry".
    let (adv_status, adv_body) = post_json(
        pool,
        &format!("/instances/{id}/advance"),
        json!({ "capability": "register_payment" }),
    )
    .await;

    assert_eq!(adv_status, StatusCode::CONFLICT, "body={adv_body}");
    let env: ErrorEnvelope = serde_json::from_value(adv_body).expect("must be ErrorEnvelope");
    assert_eq!(env.error.code, ErrorCode::Validation);
}

// ─── Helper: walk an order_to_cash instance to "fulfillment" ─────────────────

/// Walk inquiry → so_open → credit_check → confirmed → fulfillment.
/// All are non-posting transitions (no amounts needed).
async fn walk_to_fulfillment(pool: PgPool) -> i64 {
    let (s, b) = post_json(
        pool.clone(),
        "/instances",
        json!({ "process": "order_to_cash" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "start: {b}");
    let id = b["id"].as_i64().expect("id");

    for cap in &[
        "confirm_order",
        "run_credit_check",
        "release_credit_hold",
        "allocate_inventory",
    ] {
        let (s, b) = post_json(
            pool.clone(),
            &format!("/instances/{id}/advance"),
            json!({ "capability": cap }),
        )
        .await;
        assert_eq!(s, StatusCode::OK, "advance {cap}: {b}");
    }

    id
}
