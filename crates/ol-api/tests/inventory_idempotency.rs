//! Idempotency and audit coverage for `POST /inventory/receive` (#90).
//!
//! Before this, a retried receipt created a second `stock_moves` row and wrote no
//! audit row at all. Reproduced by hand against a live database on 2026-08-14
//! (two identical requests → `move_id` 1 and 2, on-hand 20 for 10 received);
//! these tests make that permanent.

use ol_api::{ReceiveStockRequest, receive_stock_core};
use sqlx::PgPool;
use uuid::Uuid;

fn receipt(key: Uuid, qty: &str) -> ReceiveStockRequest {
    ReceiveStockRequest {
        idempotency_key: key,
        sku: "WIDGET-A".to_string(),
        location_code: "MAIN".to_string(),
        qty: qty.to_string(),
        unit_cost: Some("250".to_string()),
        actor: "test".to_string(),
    }
}

async fn move_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM stock_moves")
        .fetch_one(pool)
        .await
        .expect("count stock_moves")
}

/// GIVEN a stock receipt that has already been recorded,
/// WHEN the same request is submitted again with the same idempotency key,
/// THEN exactly one `stock_moves` row exists and the second call reports
/// `replayed = true` with the original `move_id`.
#[sqlx::test(migrations = "../../migrations")]
async fn same_key_twice_creates_one_move_and_reports_replayed(pool: PgPool) {
    let key = Uuid::new_v4();

    let first = receive_stock_core(&pool, receipt(key, "10"))
        .await
        .expect("first receipt must succeed");
    assert!(!first.replayed, "the first receipt is not a replay");

    let second = receive_stock_core(&pool, receipt(key, "10"))
        .await
        .expect("the retry must succeed, not error");

    assert!(
        second.replayed,
        "the second receipt must report replayed = true"
    );
    assert_eq!(
        second.move_id, first.move_id,
        "the replay must return the ORIGINAL move_id, not a new one"
    );
    assert_eq!(
        move_count(&pool).await,
        1,
        "exactly one stock move must exist"
    );
}

/// GIVEN two genuinely different receipts,
/// WHEN both are submitted with different idempotency keys,
/// THEN two `stock_moves` rows exist.
///
/// Guards the opposite failure: a key so stable that distinct receipts collapse.
#[sqlx::test(migrations = "../../migrations")]
async fn two_different_keys_create_two_moves(pool: PgPool) {
    receive_stock_core(&pool, receipt(Uuid::new_v4(), "10"))
        .await
        .expect("first receipt");
    receive_stock_core(&pool, receipt(Uuid::new_v4(), "10"))
        .await
        .expect("second receipt");

    assert_eq!(
        move_count(&pool).await,
        2,
        "two distinct receipts, two moves"
    );
}

/// GIVEN a stock receipt,
/// WHEN it is recorded,
/// THEN the actor is persisted in the audit log and is readable.
///
/// Before the fix `events` had ZERO rows for any stock movement — a duplicate
/// receipt was not merely unattributable, it was invisible.
#[sqlx::test(migrations = "../../migrations")]
async fn a_receipt_writes_an_attributable_audit_event(pool: PgPool) {
    let result = receive_stock_core(&pool, receipt(Uuid::new_v4(), "7"))
        .await
        .expect("receipt must succeed");

    let (actor, capability, entity_id): (String, String, Option<String>) = sqlx::query_as(
        "SELECT actor, capability, entity_id FROM events WHERE capability = 'receive_stock'",
    )
    .fetch_one(&pool)
    .await
    .expect("a receipt must write exactly one audit event");

    assert_eq!(actor, "test", "the caller's actor must be recorded");
    assert_eq!(capability, "receive_stock");
    assert_eq!(
        entity_id,
        Some(result.move_id.to_string()),
        "the audit event must point at the stock move it describes"
    );
}

/// GIVEN a replayed receipt,
/// WHEN the retry is served from the stored result,
/// THEN no second audit event is written.
///
/// A replay is not a second business event; recording one would inflate the audit
/// log with work that never happened.
#[sqlx::test(migrations = "../../migrations")]
async fn a_replay_does_not_write_a_second_audit_event(pool: PgPool) {
    let key = Uuid::new_v4();
    receive_stock_core(&pool, receipt(key, "10"))
        .await
        .expect("first");
    receive_stock_core(&pool, receipt(key, "10"))
        .await
        .expect("replay");

    let events: i64 =
        sqlx::query_scalar("SELECT count(*) FROM events WHERE capability = 'receive_stock'")
            .fetch_one(&pool)
            .await
            .expect("count events");

    assert_eq!(events, 1, "one receipt happened, so one audit event");
}

/// GIVEN a receipt already recorded,
/// WHEN the same key is submitted over HTTP,
/// THEN the status is 200, not 201 — nothing was created.
#[sqlx::test(migrations = "../../migrations")]
async fn a_replayed_receipt_returns_200_not_201(pool: PgPool) {
    use tower::ServiceExt;
    let key = Uuid::new_v4();
    let body = serde_json::json!({
        "idempotency_key": key,
        "sku": "WIDGET-A",
        "location_code": "MAIN",
        "qty": "10",
        "unit_cost": "250",
        "actor": "test"
    });

    let mut statuses = Vec::new();
    for _ in 0..2 {
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/inventory/receive")
            .header("content-type", "application/json")
            .body(axum::body::Body::from(body.to_string()))
            .expect("build request");
        let response = ol_api::app(pool.clone())
            .oneshot(request)
            .await
            .expect("call endpoint");
        statuses.push(response.status());
    }

    assert_eq!(
        statuses[0],
        axum::http::StatusCode::CREATED,
        "the first call creates"
    );
    assert_eq!(
        statuses[1],
        axum::http::StatusCode::OK,
        "the replay creates nothing"
    );
}
