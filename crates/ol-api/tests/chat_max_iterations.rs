//! The chat loop must not report a truncated run as a completed one (#88).
//!
//! Until now the Ollama tool loop had NO automated coverage — the module doc
//! said so — because it needs a daemon. It does not: it needs something that
//! speaks `/api/chat`. This test stands up a mock that always answers with a
//! tool call, so the 6-iteration cap is reached deterministically.
//!
//! ONE test per binary on purpose: the handler reads `OLLAMA_URL` from the
//! process environment, so two tests mutating it in one binary would race.

use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

/// A mock Ollama that never stops asking for tool calls.
async fn always_calls_a_tool() -> Json<Value> {
    Json(json!({
        "message": {
            "role": "assistant",
            "content": "",
            "tool_calls": [{
                "function": {
                    "name": "get_account_balance",
                    "arguments": { "account_code": "1000" }
                }
            }]
        }
    }))
}

/// GIVEN a model that emits a tool call on every turn,
/// WHEN the iteration cap is hit,
/// THEN the response reports `finish_reason: "max_iterations"` and the reply
/// does not claim the work was completed.
#[sqlx::test(migrations = "../../migrations")]
async fn truncated_run_is_not_reported_as_completed(pool: PgPool) {
    let mock = Router::new().route("/api/chat", post(always_calls_a_tool));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock ollama");
    let addr = listener.local_addr().expect("mock addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, mock).await;
    });

    // SAFETY: this binary contains exactly one test, so nothing races here.
    unsafe { std::env::set_var("OLLAMA_URL", format!("http://{addr}")) };

    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/chat")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            json!({ "message": "post something for me", "history": [] }).to_string(),
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
        axum::http::StatusCode::OK,
        "a truncated run still returns 200 — the work that landed is real, body={body}"
    );
    assert_eq!(
        body["finish_reason"], "max_iterations",
        "the run was cut off by the cap and must say so"
    );

    let reply = body["reply"].as_str().expect("reply must be a string");
    assert!(
        !reply.contains("I completed the requested operations"),
        "the canned success string must never be used for a truncated run: {reply}"
    );
    assert!(
        reply.contains("ran out of steps"),
        "the reply must state that the run was cut off, got: {reply}"
    );

    assert_eq!(
        body["actions"].as_array().expect("actions array").len(),
        6,
        "every attempted tool call is still reported, so the user can see what landed"
    );
}
