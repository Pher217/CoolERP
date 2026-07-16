//! ol-api binary — reads DATABASE_URL, builds the connection pool, and serves.

use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await?;

    // Default to localhost. This server has NO authentication yet (ADR-006 OAuth 2.1
    // PKCE is designed but not wired), so it must not be reachable off-host by default.
    // Set BIND_ADDR explicitly to expose it — and only behind auth / a trusted proxy.
    let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1:3000".to_string());
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;

    tracing::warn!(
        "SECURITY: running WITHOUT authentication (dev mode). Every request is trusted \
         and the audit-log `actor` is client-asserted. Do not expose this on an untrusted \
         network. Auth wiring is tracked as the top launch issue (ADR-006)."
    );
    if !bind_addr.starts_with("127.") && !bind_addr.starts_with("localhost") {
        tracing::warn!(
            "SECURITY: BIND_ADDR={bind_addr} is not localhost — an unauthenticated API is \
             now reachable off-host. Put it behind auth and a trusted proxy."
        );
    }
    tracing::info!("listening on {bind_addr}");
    tracing::info!("OpenAPI spec at http://{bind_addr}/api-docs/openapi.json");

    let router = ol_api::app(pool);
    axum::serve(listener, router).await?;

    Ok(())
}
