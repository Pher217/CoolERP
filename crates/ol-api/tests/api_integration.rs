//! Integration tests for ol-api.
//!
//! Each test gets a fresh migrated database via `#[sqlx::test]`.  Migration
//! `0002_seed_chart_of_accounts.sql` populates the COA and the `GEN` journal,
//! so accounts 1000/4000 and journal `GEN` are always present.
//!
//! The router is exercised through `tower::ServiceExt::oneshot` — no TCP
//! listener required.

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

fn balanced_entry(idempotency_key: &str) -> Value {
    json!({
        "idempotency_key": idempotency_key,
        "journal_code": "GEN",
        "entry_date": "2025-01-15",
        "memo": "integration test",
        "actor": "test",
        "lines": [
            { "account_code": "1000", "debit": 10000, "credit": 0 },
            { "account_code": "4000", "debit": 0,     "credit": 10000 }
        ]
    })
}

// ─── GET /health ──────────────────────────────────────────────────────────────

#[sqlx::test(migrations = "../../migrations")]
async fn health_returns_200(pool: PgPool) {
    let (status, body) = get(pool, "/health").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
}

// ─── POST /journal-entries ────────────────────────────────────────────────────

#[sqlx::test(migrations = "../../migrations")]
async fn balanced_post_returns_201(pool: PgPool) {
    let key = "550e8400-e29b-41d4-a716-446655440001";
    let (status, body) = post_json(pool, "/journal-entries", balanced_entry(key)).await;
    assert_eq!(status, StatusCode::CREATED, "body={body}");
    assert_eq!(body["balanced"], true);
    assert_eq!(body["replayed"], false);
    assert!(body["entry_id"].is_i64());
}

#[sqlx::test(migrations = "../../migrations")]
async fn unbalanced_post_returns_422_with_error_envelope(pool: PgPool) {
    let payload = json!({
        "idempotency_key": "550e8400-e29b-41d4-a716-446655440002",
        "journal_code": "GEN",
        "entry_date": "2025-01-15",
        "actor": "test",
        "lines": [
            { "account_code": "1000", "debit": 500, "credit": 0 },
            { "account_code": "4000", "debit": 0,   "credit": 999 }
        ]
    });
    let (status, body) = post_json(pool, "/journal-entries", payload).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "body={body}");
    let env: ErrorEnvelope = serde_json::from_value(body).expect("must be ErrorEnvelope");
    assert_eq!(env.error.code, ErrorCode::UnbalancedEntry);
}

// ─── GET /accounts/{code}/balance ─────────────────────────────────────────────

#[sqlx::test(migrations = "../../migrations")]
async fn get_balance_after_post(pool: PgPool) {
    // Post an entry first.
    let key = "550e8400-e29b-41d4-a716-446655440003";
    let (status, _) = post_json(pool.clone(), "/journal-entries", balanced_entry(key)).await;
    assert_eq!(status, StatusCode::CREATED);

    // Balance check — needs a separate router call with the same pool.
    let (status, body) = get(pool, "/accounts/1000/balance").await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    assert_eq!(body["account_code"], "1000");
    assert_eq!(body["debits"], 10000);
    assert_eq!(body["credits"], 0);
    // 1000 is an asset — debit-positive balance.
    assert_eq!(body["balance"], 10000);
}

#[sqlx::test(migrations = "../../migrations")]
async fn unknown_account_balance_returns_404_with_error_envelope(pool: PgPool) {
    let (status, body) = get(pool, "/accounts/9999/balance").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body={body}");
    let env: ErrorEnvelope = serde_json::from_value(body).expect("must be ErrorEnvelope");
    assert_eq!(env.error.code, ErrorCode::AccountNotFound);
}

// ─── GET /processes ───────────────────────────────────────────────────────────

#[sqlx::test(migrations = "../../migrations")]
async fn list_processes_returns_customer_invoice(pool: PgPool) {
    let (status, body) = get(pool, "/processes").await;
    assert_eq!(status, StatusCode::OK, "body={body}");
    let names = body["processes"].as_array().expect("must be array");
    assert!(
        names.iter().any(|n| n == "customer_invoice"),
        "expected customer_invoice in {names:?}"
    );
}

// ─── GET /processes/{name} ────────────────────────────────────────────────────

#[sqlx::test(migrations = "../../migrations")]
async fn get_customer_invoice_process_returns_200_with_mermaid(pool: PgPool) {
    let (status, body) = get(pool, "/processes/customer_invoice").await;
    assert_eq!(status, StatusCode::OK, "body={body}");

    assert_eq!(body["process"], "customer_invoice");

    let states = body["states"].as_array().expect("states must be array");
    assert!(!states.is_empty(), "states must not be empty");

    let mermaid = body["mermaid"].as_str().expect("mermaid must be string");
    assert!(
        mermaid.contains("stateDiagram-v2"),
        "mermaid must contain stateDiagram-v2"
    );
}

#[sqlx::test(migrations = "../../migrations")]
async fn unknown_process_returns_404_with_error_envelope(pool: PgPool) {
    let (status, body) = get(pool, "/processes/does_not_exist").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body={body}");
    let env: ErrorEnvelope = serde_json::from_value(body).expect("must be ErrorEnvelope");
    // 404 for missing process uses ErrorCode::Validation (name not found)
    assert_eq!(env.error.code, ErrorCode::Validation);
}
