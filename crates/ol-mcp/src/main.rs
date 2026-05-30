//! ol-mcp binary — serve the MCP server over stdio.
//!
//! Reads `DATABASE_URL` from the environment, builds a Postgres pool,
//! and runs the MCP server on stdin/stdout.
//!
//! Process YAML directory defaults to `./processes/` relative to the current
//! working directory; override with `OL_PROCESSES_DIR`.

use std::path::PathBuf;

use anyhow::Context as _;
use ol_mcp::LedgerHandler;
use rmcp::serve_server;
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::{EnvFilter, fmt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Tracing goes to stderr so it does not corrupt the MCP stdio stream.
    fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL is required")?;

    let processes_dir: PathBuf = std::env::var("OL_PROCESSES_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("processes"));

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .context("failed to connect to DATABASE_URL")?;

    tracing::info!(
        processes_dir = %processes_dir.display(),
        "ol-mcp starting on stdio"
    );

    let handler = LedgerHandler::new(pool, processes_dir);

    // Serve over stdin / stdout.  The `(stdin, stdout)` tuple is automatically
    // converted to a transport by rmcp's `IntoTransport` impl.
    serve_server(handler, (tokio::io::stdin(), tokio::io::stdout()))
        .await
        .context("MCP server error")?
        .waiting()
        .await?;

    Ok(())
}
