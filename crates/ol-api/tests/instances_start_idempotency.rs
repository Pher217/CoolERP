//! Idempotency pin for STARTING a process instance.
//!
//! `POST /journal-entries` and `POST /inventory/receive` already make retries
//! safe through the `idempotency_keys` + `request_hash` mechanism (see
//! `receive_request_hash` in `src/lib.rs`). Starting a process instance does
//! not: `ol_engine::start_instance` inserts a fresh `process_instances` row on
//! every call, and the `POST /instances` handler forwards the body straight to
//! it — `StartInstanceRequest` does not even carry an `idempotency_key` field.
//! A retried start therefore creates a SECOND instance.
//!
//! These tests pin the contract the start operation must eventually satisfy, at
//! BOTH surfaces that reach it: the REST handler (`POST /instances`) and the
//! engine function (`ol_engine::start_instance`). They MUST FAIL today —
//! `start_instance` has no idempotency at all, so the "same call twice" cases
//! find two instance rows. That failure is expected and correct; it turns green
//! the moment start idempotency lands.

use axum::{
    body::Body,
    http::{Request, StatusCode, header},
};
use ol_sdk::{ErrorCode, ErrorEnvelope};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

// ─── Helpers ──────────────────────────────────────────────────────────────────

async fn post_json(pool: PgPool, path: &str, payload: Value) -> (StatusCode, Value) {
    let router = ol_api::app(pool);
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

/// Count `process_instances` rows for the given process name.
async fn instance_count(pool: &PgPool, process: &str) -> i64 {
    sqlx::query_scalar::<_, i64>("SELECT count(*) FROM process_instances WHERE process = $1")
        .bind(process)
        .fetch_one(pool)
        .await
        .expect("count process_instances")
}

// ─── POST /instances — same key, same payload ─────────────────────────────────

/// GIVEN a valid process and a caller-supplied idempotency key,
/// WHEN `POST /instances` is called twice with the SAME process, the SAME
/// payload, and the SAME idempotency_key,
/// THEN exactly ONE `process_instances` row exists (not two), and the second
/// call replays the FIRST instance's id rather than creating a new one or
/// erroring.
#[sqlx::test(migrations = "../../migrations")]
async fn start_same_key_same_payload_twice_creates_one_instance(pool: PgPool) {
    let key = Uuid::new_v4();
    let payload = json!({
        "process": "order_to_cash",
        "idempotency_key": key,
        "reference": "SO-IDEM-1",
        "context": { "customer": "ACME" },
    });

    let (first_status, first_body) = post_json(pool.clone(), "/instances", payload.clone()).await;
    assert_eq!(
        first_status,
        StatusCode::CREATED,
        "the first start must create: {first_body}"
    );
    let first_id = first_body["id"].as_i64().expect("first id must be integer");

    let (second_status, second_body) = post_json(pool.clone(), "/instances", payload).await;
    assert_eq!(
        second_status,
        StatusCode::OK,
        "the retry must replay, not create (201) and not error: {second_body}"
    );
    let second_id = second_body["id"]
        .as_i64()
        .expect("second id must be integer");
    assert_eq!(
        second_id, first_id,
        "the replay must return the ORIGINAL instance id, not a new one"
    );

    assert_eq!(
        instance_count(&pool, "order_to_cash").await,
        1,
        "exactly one instance must exist — a retry must not create a second"
    );
}

// ─── POST /instances — same key, different payload ────────────────────────────

/// GIVEN an idempotency key already used to start an instance,
/// WHEN `POST /instances` is called again with the SAME idempotency_key but a
/// DIFFERENT payload (different reference + context),
/// THEN the call is REJECTED with an `IDEMPOTENCY_KEY_REUSED`-class error
/// (`DuplicateIdempotencyKey`, HTTP 409) and NO second instance is created.
#[sqlx::test(migrations = "../../migrations")]
async fn start_same_key_different_payload_is_rejected_with_no_second_instance(pool: PgPool) {
    let key = Uuid::new_v4();

    let (first_status, first_body) = post_json(
        pool.clone(),
        "/instances",
        json!({
            "process": "order_to_cash",
            "idempotency_key": key,
            "reference": "SO-IDEM-A",
            "context": { "customer": "ACME" },
        }),
    )
    .await;
    assert_eq!(
        first_status,
        StatusCode::CREATED,
        "the first start must create: {first_body}"
    );

    // Same key, but a genuinely different economic payload.
    let (second_status, second_body) = post_json(
        pool.clone(),
        "/instances",
        json!({
            "process": "order_to_cash",
            "idempotency_key": key,
            "reference": "SO-IDEM-B",
            "context": { "customer": "GLOBEX" },
        }),
    )
    .await;

    assert_eq!(
        second_status,
        StatusCode::CONFLICT,
        "key reuse with a different payload must be rejected, not silently accepted: {second_body}"
    );
    let env: ErrorEnvelope =
        serde_json::from_value(second_body).expect("rejection must be an ErrorEnvelope");
    assert_eq!(
        env.error.code,
        ErrorCode::DuplicateIdempotencyKey,
        "the error must be the IDEMPOTENCY_KEY_REUSED class, not a generic validation error"
    );

    assert_eq!(
        instance_count(&pool, "order_to_cash").await,
        1,
        "the rejected call must not have created a second instance"
    );
}

// ─── Engine surface — the same start operation reached directly ───────────────

/// GIVEN the engine start operation (the layer `POST /instances` delegates to),
/// WHEN `ol_engine::start_instance` is called twice with the SAME process,
/// reference, and context,
/// THEN exactly ONE `process_instances` row exists and the second call returns
/// the FIRST instance's id rather than creating a new one.
///
/// This pins idempotency at the engine itself, so the property holds whether
/// the start is reached over HTTP or called directly — not merely faked in the
/// handler. `start_instance` currently has no idempotency at all, so this finds
/// two rows today and fails, as intended.
#[sqlx::test(migrations = "../../migrations")]
async fn engine_start_same_call_twice_creates_one_instance(pool: PgPool) {
    let context = json!({ "customer": "ACME" });

    let first = ol_engine::start_instance(
        &pool,
        "order_to_cash",
        Some("SO-ENG-1".to_string()),
        context.clone(),
    )
    .await
    .expect("first start must succeed");

    let second = ol_engine::start_instance(
        &pool,
        "order_to_cash",
        Some("SO-ENG-1".to_string()),
        context,
    )
    .await
    .expect("the retry must replay, not error");

    assert_eq!(
        second.id, first.id,
        "the engine must replay the original instance id, not mint a new one"
    );
    assert_eq!(
        instance_count(&pool, "order_to_cash").await,
        1,
        "exactly one instance must exist after two identical engine starts"
    );
}
