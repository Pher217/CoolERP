//! An unreachable Ollama must not be reported as a successful chat turn (#88).
//!
//! The handler previously returned `200 OK` on every path including transport
//! failure, while its OpenAPI doc advertised a `503` that could never occur.
//!
//! ONE test per binary: the handler reads `OLLAMA_URL` from the process env.

use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

/// GIVEN an unreachable Ollama,
/// WHEN POST /chat is called,
/// THEN the status is 503 and `finish_reason` is `transport_error`.
#[sqlx::test(migrations = "../../migrations")]
async fn unreachable_backend_returns_503(pool: PgPool) {
    // Port 1 on loopback: nothing listens, so the connect fails fast.
    // SAFETY: this binary contains exactly one test.
    unsafe { std::env::set_var("OLLAMA_URL", "http://127.0.0.1:1") };

    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/chat")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            json!({ "message": "hello", "history": [] }).to_string(),
        ))
        .expect("build request");

    let response = ol_api::app(pool)
        .oneshot(request)
        .await
        .expect("call /chat");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: Value = serde_json::from_slice(&bytes).expect("parse body");

    assert_eq!(
        status,
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "an unreachable backend is a 503, matching the published contract, body={body}"
    );
    assert_eq!(body["finish_reason"], "transport_error");
    assert_eq!(
        body["actions"].as_array().expect("actions").len(),
        0,
        "nothing ran, so nothing is reported as having run"
    );
}
