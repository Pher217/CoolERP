//! ol-mcp — MCP server exposing OpenERP's typed, idempotent capabilities.
//!
//! Discrete named capabilities only — NO admin shell, NO raw-SQL `execute` tool.
//!
//! Auth:
//! TODO: OAuth 2.1 PKCE (ADR-006). Running unauthenticated for now.
//! - OAuth 2.1 + mandatory PKCE (S256) for public clients; no implicit flow.
//! - RFC 9728 Protected Resource Metadata at /.well-known/oauth-protected-resource (MUST).
//! - Scoped tokens: ledger:read, ledger:post, ar:invoice, ap:bill.

use std::path::PathBuf;

use chrono::NaiveDate;
use ol_domain::Line;
use ol_ledger::{Balance, PostError, PostRequest, PostResult};
use ol_process::Process;
use ol_sdk::{ApiError, ErrorCode};
use rmcp::{
    Json, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerInfo},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Parameter types
// ---------------------------------------------------------------------------

/// A single journal line for a `post_journal_entry` call.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LineParam {
    /// Account code (e.g. "1000").
    pub account_code: String,
    /// Debit amount in integer cents. Exactly one of debit/credit must be non-zero.
    pub debit: i64,
    /// Credit amount in integer cents. Exactly one of debit/credit must be non-zero.
    pub credit: i64,
}

/// Parameters for `post_journal_entry`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PostJournalEntryParams {
    /// UUID idempotency key — reuse the same key to replay without double-posting.
    pub idempotency_key: Uuid,
    /// Journal code (e.g. "GJ" for General Journal).
    pub journal_code: String,
    /// ISO date of the entry (YYYY-MM-DD).
    pub entry_date: String,
    /// Economic date determining the fiscal period (YYYY-MM-DD). Defaults to entry_date when absent.
    pub effective_date: Option<String>,
    /// Optional memo / narrative.
    pub memo: Option<String>,
    /// Optional external reference (invoice number, etc.).
    pub reference: Option<String>,
    /// Actor identifier recorded in the audit log (e.g. the MCP token subject).
    pub actor: String,
    /// Entry lines — minimum 2, balanced (Σdebit = Σcredit).
    pub lines: Vec<LineParam>,
}

/// Parameters for `get_account_balance`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetAccountBalanceParams {
    /// Account code (e.g. "1000").
    pub account_code: String,
}

/// Parameters for `get_process`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetProcessParams {
    /// Process name as it appears in the YAML `process:` field (e.g. "customer_invoice").
    pub name: String,
}

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// Result of `post_journal_entry`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PostJournalEntryResult {
    pub entry_id: i64,
    pub balanced: bool,
    pub replayed: bool,
}

/// Result of `get_account_balance`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AccountBalanceResult {
    pub account_code: String,
    /// Raw debit sum (integer cents).
    pub debits: i64,
    /// Raw credit sum (integer cents).
    pub credits: i64,
    /// Signed balance in integer cents (debit-positive for assets/expenses;
    /// credit-positive for liabilities/equity/income).
    pub balance: i64,
}

/// A single process entry returned by `list_processes`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProcessSummary {
    pub name: String,
    pub states: Vec<String>,
}

/// Result of `list_processes`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListProcessesResult {
    pub processes: Vec<ProcessSummary>,
}

/// Result of `get_process`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetProcessResult {
    pub name: String,
    pub states: Vec<String>,
    /// Mermaid `stateDiagram-v2` diagram of this process.
    pub mermaid: String,
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

/// MCP server: holds a Postgres pool and the path to the process YAML directory.
#[derive(Clone)]
pub struct LedgerHandler {
    pool: PgPool,
    processes_dir: PathBuf,
    // Consumed by the rmcp tool_handler macro at dispatch time.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl LedgerHandler {
    /// Construct the handler.
    ///
    /// `processes_dir` is the directory containing `.yaml` process definition files.
    pub fn new(pool: PgPool, processes_dir: PathBuf) -> Self {
        Self {
            pool,
            processes_dir,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl LedgerHandler {
    /// Post a balanced journal entry idempotently.
    ///
    /// Returns the entry ID. Re-submitting the same `idempotency_key`
    /// returns `replayed = true` instead of creating a duplicate.
    #[tool(name = "post_journal_entry")]
    pub async fn post_journal_entry(
        &self,
        Parameters(params): Parameters<PostJournalEntryParams>,
    ) -> Result<Json<PostJournalEntryResult>, String> {
        let entry_date = NaiveDate::parse_from_str(&params.entry_date, "%Y-%m-%d")
            .map_err(|e| format!("VALIDATION: invalid entry_date (expected YYYY-MM-DD): {e}"))?;

        let effective_date = params
            .effective_date
            .as_deref()
            .map(|s| {
                NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| {
                    format!("VALIDATION: invalid effective_date (expected YYYY-MM-DD): {e}")
                })
            })
            .transpose()?;

        let lines: Vec<Line> = params
            .lines
            .into_iter()
            .map(|l| Line {
                account_code: l.account_code,
                debit: l.debit,
                credit: l.credit,
            })
            .collect();

        let req = PostRequest {
            idempotency_key: params.idempotency_key,
            journal_code: params.journal_code,
            entry_date,
            effective_date,
            memo: params.memo,
            reference: params.reference,
            actor: params.actor,
            lines,
        };

        ol_ledger::post_journal_entry(&self.pool, &req)
            .await
            .map(
                |PostResult {
                     entry_id,
                     balanced,
                     replayed,
                 }| {
                    Json(PostJournalEntryResult {
                        entry_id,
                        balanced,
                        replayed,
                    })
                },
            )
            .map_err(post_error_to_string)
    }

    /// Return the signed balance of an account.
    ///
    /// Debits and credits are raw sums in integer cents. The `balance` field is
    /// type-normalized: debit-positive for assets and expenses; credit-positive
    /// for liabilities, equity, and income.
    #[tool(name = "get_account_balance")]
    pub async fn get_account_balance(
        &self,
        Parameters(params): Parameters<GetAccountBalanceParams>,
    ) -> Result<Json<AccountBalanceResult>, String> {
        ol_ledger::account_balance(&self.pool, &params.account_code)
            .await
            .map(
                |Balance {
                     account_code,
                     debits,
                     credits,
                     balance,
                 }| {
                    Json(AccountBalanceResult {
                        account_code,
                        debits,
                        credits,
                        balance,
                    })
                },
            )
            .map_err(post_error_to_string)
    }

    /// List all available business process definitions.
    #[tool(name = "list_processes")]
    pub async fn list_processes(&self) -> Result<Json<ListProcessesResult>, String> {
        Process::load_dir(&self.processes_dir)
            .map(|processes| {
                Json(ListProcessesResult {
                    processes: processes
                        .into_iter()
                        .map(|p| ProcessSummary {
                            name: p.process.clone(),
                            states: p.states.clone(),
                        })
                        .collect(),
                })
            })
            .map_err(|e| format!("INTERNAL: {e}"))
    }

    /// Get a business process definition by name, including its Mermaid state diagram.
    #[tool(name = "get_process")]
    pub async fn get_process(
        &self,
        Parameters(params): Parameters<GetProcessParams>,
    ) -> Result<Json<GetProcessResult>, String> {
        let processes =
            Process::load_dir(&self.processes_dir).map_err(|e| format!("INTERNAL: {e}"))?;

        let process = processes
            .into_iter()
            .find(|p| p.process == params.name)
            .ok_or_else(|| format!("INTERNAL: process '{}' not found", params.name))?;

        let mermaid = process.to_mermaid(None);
        Ok(Json(GetProcessResult {
            name: process.process.clone(),
            states: process.states.clone(),
            mermaid,
        }))
    }
}

/// Map a [`PostError`] to a structured `"CODE: message"` string for MCP tool errors.
fn post_error_to_string(e: PostError) -> String {
    let api_err = match &e {
        PostError::Domain(d) => ApiError::new(ErrorCode::UnbalancedEntry, d.to_string()),
        PostError::AccountNotFound(c) => ApiError::new(
            ErrorCode::AccountNotFound,
            format!("account not found: {c}"),
        ),
        PostError::JournalNotFound(c) => ApiError::new(
            ErrorCode::JournalNotFound,
            format!("journal not found: {c}"),
        ),
        PostError::Unbalanced { message } => {
            ApiError::new(ErrorCode::UnbalancedEntry, message.clone())
        }
        PostError::AppendOnly { message } => ApiError::new(ErrorCode::AppendOnly, message.clone()),
        PostError::PeriodClosed { message } => {
            ApiError::new(ErrorCode::PeriodClosed, message.clone())
        }
        PostError::PeriodOverlap { message } => {
            ApiError::new(ErrorCode::Validation, message.clone())
        }
        PostError::PeriodNotFound(code) => ApiError::new(
            ErrorCode::Validation,
            format!("fiscal period not found: {code}"),
        ),
        PostError::Serialization(n) => ApiError::new(
            ErrorCode::SerializationFailure,
            format!("exhausted {n} retry attempts"),
        ),
        PostError::Db(db) => ApiError::new(ErrorCode::Internal, db.to_string()),
    };
    api_err.to_string()
}

#[tool_handler]
impl ServerHandler for LedgerHandler {
    fn get_info(&self) -> ServerInfo {
        use rmcp::model::ServerCapabilities;
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        ServerInfo::new(capabilities)
            .with_server_info(Implementation::new("ol-mcp", env!("CARGO_PKG_VERSION")))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// The tool router must list exactly the four expected tools.
    /// This does not require a live database or MCP client.
    #[test]
    fn tool_router_lists_expected_tools() {
        let router = LedgerHandler::tool_router();
        let tools = router.list_all();
        let names: std::collections::HashSet<&str> =
            tools.iter().map(|t| t.name.as_ref()).collect();

        assert!(
            names.contains("post_journal_entry"),
            "missing post_journal_entry"
        );
        assert!(
            names.contains("get_account_balance"),
            "missing get_account_balance"
        );
        assert!(names.contains("list_processes"), "missing list_processes");
        assert!(names.contains("get_process"), "missing get_process");
        assert_eq!(names.len(), 4, "unexpected extra tools: {names:?}");
    }
}
