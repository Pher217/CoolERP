//! Export the actual router's OpenAPI response, without connecting to a database.
use axum::{body::Body, http::Request};
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let pool = PgPoolOptions::new().connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")?;
    let response = ol_api::app(pool)
        .oneshot(Request::get("/api-docs/openapi.json").body(Body::empty())?)
        .await?;
    anyhow::ensure!(response.status().is_success(), "OpenAPI route failed");
    let bytes = axum::body::to_bytes(response.into_body(), 10 * 1024 * 1024).await?;
    let document: serde_json::Value = serde_json::from_slice(&bytes)?;
    println!("{}", serde_json::to_string_pretty(&document)?);
    Ok(())
}
