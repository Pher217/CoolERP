//! A normal chat exchange reports `finish_reason: completed` (#88).
//!
//! ONE test per binary: the handler reads `OLLAMA_URL` from the process env.

use axum::{Json, Router, routing::post};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

/// A mock Ollama that answers with final text and no tool calls.
async fn answers_with_text() -> Json<Value> {
    Json(json!({
        "message": { "role": "assistant", "content": "Account 1000 is at 0.", "tool_calls": [] }
    }))
}

/// GIVEN a model that returns text,
/// WHEN POST /chat is called,
/// THEN `finish_reason` is `completed` and the model's own words are the reply.
#[sqlx::test(migrations = "../../migrations")]
async fn a_text_answer_completes(pool: PgPool) {
    let mock = Router::new().route("/api/chat", post(answers_with_text));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let _ = axum::serve(listener, mock).await;
    });

    // SAFETY: this binary contains exactly one test.
    unsafe { std::env::set_var("OLLAMA_URL", format!("http://{addr}")) };

    let request = axum::http::Request::builder()
        .method("POST")
        .uri("/chat")
        .header("content-type", "application/json")
        .body(axum::body::Body::from(
            json!({ "message": "what is account 1000?", "history": [] }).to_string(),
        ))
        .expect("build request");

    let response = ol_api::app(pool)
        .oneshot(request)
        .await
        .expect("call /chat");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body: Value = serde_json::from_slice(&bytes).expect("parse body");

    assert_eq!(body["finish_reason"], "completed");
    assert_eq!(
        body["reply"], "Account 1000 is at 0.",
        "the model's own answer is returned, not a substitute"
    );
}
