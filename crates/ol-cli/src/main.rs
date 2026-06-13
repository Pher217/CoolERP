//! ol — OpenERP CLI. (AGPL-3.0)
//!
//! Subcommands: migrate, post, balance, serve (stub), replay (stub).
//! Reads DATABASE_URL from the environment.

use chrono::NaiveDate;
use clap::{Parser, Subcommand};
use ol_domain::Line;
use ol_ledger::{PostRequest, account_balance, post_journal_entry};
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "ol", about = "OpenERP CLI")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run all pending SQL migrations against DATABASE_URL.
    Migrate,

    /// Post a balanced journal entry.
    Post {
        /// Journal code (default: GEN).
        #[arg(short, long, default_value = "GEN")]
        journal: String,

        /// Entry date in YYYY-MM-DD format (required).
        #[arg(long)]
        date: String,

        /// Optional free-text memo.
        #[arg(long)]
        memo: Option<String>,

        /// Audit actor (default: cli).
        #[arg(long, default_value = "cli")]
        actor: String,

        /// Journal line in CODE:DEBIT:CREDIT format (cents). Repeat for each line.
        /// Example: --line 1000:10000:0 --line 4000:0:10000
        #[arg(long = "line", required = true)]
        lines: Vec<String>,
    },

    /// Print the balance for an account code.
    Balance {
        /// Account code, e.g. 1000.
        code: String,
    },

    /// Start the MCP + REST server (not yet implemented).
    Serve,

    /// Rebuild projections from the event log (not yet implemented).
    Replay,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    match cli.command {
        Cmd::Serve => {
            eprintln!("ol serve: not yet implemented (Stage 2)");
            std::process::exit(1);
        }
        Cmd::Replay => {
            eprintln!("ol replay: not yet implemented (Stage 2)");
            std::process::exit(1);
        }
        Cmd::Migrate => {
            let pool = connect().await;
            match sqlx::migrate!("../../migrations").run(&pool).await {
                Ok(()) => println!("migrations applied"),
                Err(e) => {
                    eprintln!("migrate error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Cmd::Post {
            journal,
            date,
            memo,
            actor,
            lines: raw_lines,
        } => {
            let entry_date = match date.parse::<NaiveDate>() {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("invalid --date '{date}': {e}");
                    std::process::exit(1);
                }
            };

            let mut lines: Vec<Line> = Vec::with_capacity(raw_lines.len());
            for raw in &raw_lines {
                match parse_line(raw) {
                    Ok(l) => lines.push(l),
                    Err(msg) => {
                        eprintln!("invalid --line '{raw}': {msg}");
                        std::process::exit(1);
                    }
                }
            }

            let pool = connect().await;
            let req = PostRequest {
                idempotency_key: Uuid::new_v4(),
                journal_code: journal,
                entry_date,
                effective_date: None,
                memo,
                reference: None,
                actor,
                lines,
            };

            match post_journal_entry(&pool, &req).await {
                Ok(result) => {
                    println!("entry_id={}", result.entry_id);
                    println!("balanced={}", result.balanced);
                    println!("replayed={}", result.replayed);
                }
                Err(e) => {
                    eprintln!("post error: {e}");
                    std::process::exit(1);
                }
            }
        }
        Cmd::Balance { code } => {
            let pool = connect().await;
            match account_balance(&pool, &code).await {
                Ok(b) => {
                    println!("account_code={}", b.account_code);
                    println!("debits={}", b.debits);
                    println!("credits={}", b.credits);
                    println!("balance={}", b.balance);
                }
                Err(e) => {
                    eprintln!("balance error: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
}

/// Parse a line spec "CODE:DEBIT:CREDIT[:CURRENCY]" into a [`Line`].
/// Debit and credit are integer cents (i64). Currency defaults to "EUR" when absent.
fn parse_line(s: &str) -> Result<Line, String> {
    let parts: Vec<&str> = s.splitn(4, ':').collect();
    if parts.len() < 3 {
        return Err("expected CODE:DEBIT:CREDIT[:CURRENCY]".into());
    }
    let account_code = parts[0].trim().to_string();
    if account_code.is_empty() {
        return Err("account code is empty".into());
    }
    let debit: i64 = parts[1]
        .trim()
        .parse()
        .map_err(|e| format!("debit is not an integer: {e}"))?;
    let credit: i64 = parts[2]
        .trim()
        .parse()
        .map_err(|e| format!("credit is not an integer: {e}"))?;
    let currency = parts
        .get(3)
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "EUR".into());
    Ok(Line::in_currency(account_code, debit, credit, currency))
}

/// Connect to Postgres using DATABASE_URL from the environment.
/// Exits with a message on failure.
async fn connect() -> sqlx::PgPool {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        eprintln!("DATABASE_URL is not set");
        std::process::exit(1);
    });
    PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .unwrap_or_else(|e| {
            eprintln!("could not connect to database: {e}");
            std::process::exit(1);
        })
}
