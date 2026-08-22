//! ol-mcp — MCP server exposing CoolERP's typed, idempotent capabilities.
//!
//! Discrete named capabilities only — NO admin shell, NO raw-SQL `execute` tool.
//!
//! Auth:
//! TODO: OAuth 2.1 PKCE (ADR-006). Running unauthenticated for now.
//! - OAuth 2.1 + mandatory PKCE (S256) for public clients; no implicit flow.
//! - RFC 9728 Protected Resource Metadata at /.well-known/oauth-protected-resource (MUST).
//! - Scoped tokens: ledger:read, ledger:post, ar:invoice, ap:bill.

use std::{collections::HashMap, path::PathBuf};

use chrono::{Local, NaiveDate};
use ol_domain::Line;
use ol_engine::AdvanceInput;
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
use serde_json::Value;
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
    /// ISO-4217 currency code (e.g. "EUR", "USD"). Defaults to "EUR" when absent.
    pub currency: Option<String>,
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

/// Parameters for `start_process`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StartProcessParams {
    /// UUID idempotency key — reuse the same key to replay without creating a
    /// duplicate instance.
    pub idempotency_key: Uuid,
    /// Process name, e.g. "order_to_cash".
    pub process: String,
    /// Optional external reference (order number, customer id, etc.).
    pub reference: Option<String>,
    /// Optional initial context as a JSON object.
    pub context: Option<Value>,
}

/// Parameters for `advance_process`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AdvanceProcessParams {
    /// Instance id returned by `start_process`.
    pub instance_id: i64,
    /// Capability (transition label) to execute, e.g. "confirm_order".
    pub capability: String,
    /// Integer-cents amounts for posting steps. Include exactly the keys listed in
    /// `posting.required_amount_keys` (one key per credited role when there is more
    /// than one credit role, plus the key 'amount' = the debit total).
    /// Non-posting steps need no amounts.
    #[serde(default)]
    pub amounts: HashMap<String, i64>,
    /// JSON object merged into the instance context (shallow merge).
    pub context_patch: Option<Value>,
}

/// Parameters for `get_process_instance`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetProcessInstanceParams {
    /// Instance id.
    pub instance_id: i64,
}

/// Parameters for `list_process_instances`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListProcessInstancesParams {
    /// Filter by process name, e.g. "order_to_cash".
    pub process: Option<String>,
    /// Filter by status: "active", "completed", or "cancelled".
    pub status: Option<String>,
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
    /// ISO-4217 currency code (e.g. "EUR").
    pub currency: String,
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

/// A process instance snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProcessInstanceResult {
    pub id: i64,
    pub process: String,
    pub current_state: String,
    pub status: String,
    pub reference: Option<String>,
    pub context: Value,
}

impl From<ol_engine::Instance> for ProcessInstanceResult {
    fn from(i: ol_engine::Instance) -> Self {
        Self {
            id: i.id,
            process: i.process,
            current_state: i.current_state,
            status: i.status,
            reference: i.reference,
            context: i.context,
        }
    }
}

/// One step-log row.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StepLogResult {
    pub id: i64,
    pub from_state: String,
    pub to_state: String,
    pub capability: String,
    pub actor: String,
    pub entry_id: Option<i64>,
}

/// Posting amounts required by an available transition.
///
/// `required_amount_keys` lists exactly the keys the caller must supply in the
/// `amounts` map. The `credit_roles` field describes every account role this
/// transition credits and is always populated.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PostingRequirementResult {
    /// The debit role name (informational; the caller uses key `"amount"`).
    pub debit_role: String,
    /// Every account role this transition credits, always populated.
    pub credit_roles: Vec<String>,
    /// Exactly the keys the caller must supply in `amounts`.
    pub required_amount_keys: Vec<String>,
}

/// One legal next move from the instance's current state.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AvailableTransitionResult {
    /// Pass this capability name to `advance_process`.
    pub capability: String,
    /// State the instance will enter when this transition fires.
    pub to_state: String,
    /// `Some` when posting amounts are required; `None` for non-posting transitions.
    pub posting: Option<PostingRequirementResult>,
}

/// Result of `get_process_instance` — instance, full step log, and available next moves.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GetProcessInstanceResult {
    pub instance: ProcessInstanceResult,
    pub steps: Vec<StepLogResult>,
    /// Legal next capabilities from the current state.  Empty for terminal states.
    pub available: Vec<AvailableTransitionResult>,
}

/// Result of `list_process_instances`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListProcessInstancesResult {
    pub instances: Vec<ProcessInstanceResult>,
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

/// The tool contract this server advertises to MCP clients.
///
/// Exposed so that the tool surface can be inspected without standing up a
/// server — used by the snapshot test below and by the cross-surface drift test
/// in `ol-api` (issue #89), which checks that the chat manifest and this router
/// have not diverged. The `tool_router()` the macro generates is private.
pub fn advertised_tools() -> Vec<rmcp::model::Tool> {
    let mut tools = LedgerHandler::tool_router().list_all();
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    tools
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
            .map(|l| {
                Line::in_currency(
                    l.account_code,
                    l.debit,
                    l.credit,
                    l.currency.unwrap_or_else(|| "EUR".into()),
                )
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
                     currency,
                     debits,
                     credits,
                     balance,
                 }| {
                    Json(AccountBalanceResult {
                        account_code,
                        currency,
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

    /// Start a new instance of a business process.
    ///
    /// Returns the created instance. Use `advance_process` to step through transitions.
    #[tool(name = "start_process")]
    pub async fn start_process(
        &self,
        Parameters(params): Parameters<StartProcessParams>,
    ) -> Result<Json<ProcessInstanceResult>, String> {
        let context = params
            .context
            .unwrap_or_else(|| Value::Object(Default::default()));

        ol_engine::start_instance_with_key(
            &self.pool,
            &params.process,
            params.reference,
            context,
            params.idempotency_key,
        )
        .await
        .map(|outcome| Json(ProcessInstanceResult::from(outcome.instance)))
        .map_err(engine_error_to_string)
    }

    /// Advance a process instance by one transition.
    ///
    /// Always call `get_process_instance` first to read the `available` array, which
    /// lists the legal capability names from the current state.  Pass the exact
    /// capability name from `available` — never guess.
    ///
    /// For transitions that post a GL entry (`available[n].posting != null`), supply
    /// `amounts` keyed by exactly the entries in `posting.required_amount_keys`:
    /// `"amount"` = the debit total in integer cents, plus one key per credited role
    /// when the rule credits more than one (those must sum to `"amount"`).
    /// `posting.credit_roles` names the accounts being credited — descriptive, not
    /// the keys to send.  Non-posting transitions need no `amounts`.
    ///
    /// On error the message names the available capabilities from the current state.
    #[tool(name = "advance_process")]
    pub async fn advance_process(
        &self,
        Parameters(params): Parameters<AdvanceProcessParams>,
    ) -> Result<Json<ProcessInstanceResult>, String> {
        let context_patch = params
            .context_patch
            .unwrap_or_else(|| Value::Object(Default::default()));

        let input = AdvanceInput {
            amounts: params.amounts,
            context_patch,
            entry_date: Local::now().date_naive(),
        };

        ol_engine::advance_instance(
            &self.pool,
            params.instance_id,
            &params.capability,
            "mcp",
            input,
        )
        .await
        .map(|inst| Json(ProcessInstanceResult::from(inst)))
        .map_err(engine_error_to_string)
    }

    /// Get a process instance, its full step audit log, and the `available` next capabilities.
    ///
    /// The `available` array lists every legal capability name from the current state,
    /// plus the posting amounts required (if any). Always call this before `advance_process`
    /// to know the exact capability name to use — never guess.
    #[tool(name = "get_process_instance")]
    pub async fn get_process_instance(
        &self,
        Parameters(params): Parameters<GetProcessInstanceParams>,
    ) -> Result<Json<GetProcessInstanceResult>, String> {
        let result = ol_engine::get_instance(&self.pool, params.instance_id)
            .await
            .map_err(engine_error_to_string)?;

        Ok(Json(GetProcessInstanceResult {
            instance: ProcessInstanceResult::from(result.instance),
            steps: result
                .steps
                .into_iter()
                .map(|s| StepLogResult {
                    id: s.id,
                    from_state: s.from_state,
                    to_state: s.to_state,
                    capability: s.capability,
                    actor: s.actor,
                    entry_id: s.entry_id,
                })
                .collect(),
            available: result
                .available
                .into_iter()
                .map(|at| AvailableTransitionResult {
                    capability: at.capability,
                    to_state: at.to_state,
                    posting: at.posting.map(|p| PostingRequirementResult {
                        debit_role: p.debit_role,
                        credit_roles: p.credit_roles,
                        required_amount_keys: p.required_amount_keys,
                    }),
                })
                .collect(),
        }))
    }

    /// List process instances, optionally filtered by process name and/or status.
    #[tool(name = "list_process_instances")]
    pub async fn list_process_instances(
        &self,
        Parameters(params): Parameters<ListProcessInstancesParams>,
    ) -> Result<Json<ListProcessInstancesResult>, String> {
        ol_engine::list_instances(
            &self.pool,
            params.process.as_deref(),
            params.status.as_deref(),
        )
        .await
        .map(|list| {
            Json(ListProcessInstancesResult {
                instances: list.into_iter().map(ProcessInstanceResult::from).collect(),
            })
        })
        .map_err(engine_error_to_string)
    }
}

/// Map an [`ol_engine::EngineError`] to a structured `"CODE: message"` string for MCP tool errors.
fn engine_error_to_string(e: ol_engine::EngineError) -> String {
    let code = match e {
        ol_engine::EngineError::ProcessNotFound(_) => ErrorCode::Validation,
        ol_engine::EngineError::IllegalTransition { .. } => ErrorCode::Validation,
        ol_engine::EngineError::InstanceNotActive(_) => ErrorCode::Validation,
        ol_engine::EngineError::ConcurrentAdvance => ErrorCode::SerializationFailure,
        ol_engine::EngineError::IdempotencyKeyReused(_) => ErrorCode::DuplicateIdempotencyKey,
        ol_engine::EngineError::UnknownRole(_) => ErrorCode::Validation,
        ol_engine::EngineError::PostingAmountsRequired { .. } => ErrorCode::Validation,
        ol_engine::EngineError::Unbalanced { .. } => ErrorCode::UnbalancedEntry,
        ol_engine::EngineError::Ledger(inner) => return post_error_to_string(inner),
        ol_engine::EngineError::Db(_)
        | ol_engine::EngineError::Io(_)
        | ol_engine::EngineError::ProcessLoad(_) => ErrorCode::Internal,
    };
    ApiError::new(code, e.to_string()).to_string()
}

/// Build an [`ApiError`] for `code`, stripping a leading `"<CODE>: "` prefix from
/// `message` if the underlying error source already supplied it. This keeps the
/// final MCP string single-prefixed when a `PostError` variant's message already
/// contains its code.
fn api_error_for_post(code: ErrorCode, message: impl Into<String>) -> ApiError {
    let message = message.into();
    let prefix = format!("{code}: ");
    let message = message
        .strip_prefix(&prefix)
        .map(ToOwned::to_owned)
        .unwrap_or(message);
    ApiError::new(code, message)
}

/// Map a [`PostError`] to a structured `"CODE: message"` string for MCP tool errors.
fn post_error_to_string(e: PostError) -> String {
    let api_err = match &e {
        PostError::Domain(d) => api_error_for_post(ErrorCode::UnbalancedEntry, d.to_string()),
        PostError::AccountNotFound(c) => ApiError::new(
            ErrorCode::AccountNotFound,
            format!("account not found: {c}"),
        ),
        PostError::JournalNotFound(c) => ApiError::new(
            ErrorCode::JournalNotFound,
            format!("journal not found: {c}"),
        ),
        PostError::Unbalanced { message } => {
            api_error_for_post(ErrorCode::UnbalancedEntry, message)
        }
        PostError::AppendOnly { message } => api_error_for_post(ErrorCode::AppendOnly, message),
        PostError::PeriodClosed { message } => api_error_for_post(ErrorCode::PeriodClosed, message),
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
        PostError::IdempotencyKeyReused(message) => {
            api_error_for_post(ErrorCode::DuplicateIdempotencyKey, message)
        }
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

    const INPUT_ADVANCE_PROCESS: &str = r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "properties": {
    "amounts": {
      "additionalProperties": {
        "format": "int64",
        "type": "integer"
      },
      "default": {},
      "description": "Integer-cents amounts for posting steps. Include exactly the keys listed in\n`posting.required_amount_keys` (one key per credited role when there is more\nthan one credit role, plus the key 'amount' = the debit total).\nNon-posting steps need no amounts.",
      "type": "object"
    },
    "capability": {
      "description": "Capability (transition label) to execute, e.g. \"confirm_order\".",
      "type": "string"
    },
    "context_patch": {
      "description": "JSON object merged into the instance context (shallow merge)."
    },
    "instance_id": {
      "description": "Instance id returned by `start_process`.",
      "format": "int64",
      "type": "integer"
    }
  },
  "required": [
    "instance_id",
    "capability"
  ],
  "type": "object"
}"##;
    const INPUT_GET_ACCOUNT_BALANCE: &str = r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "properties": {
    "account_code": {
      "description": "Account code (e.g. \"1000\").",
      "type": "string"
    }
  },
  "required": [
    "account_code"
  ],
  "type": "object"
}"##;
    const INPUT_GET_PROCESS: &str = r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "properties": {
    "name": {
      "description": "Process name as it appears in the YAML `process:` field (e.g. \"customer_invoice\").",
      "type": "string"
    }
  },
  "required": [
    "name"
  ],
  "type": "object"
}"##;
    const INPUT_GET_PROCESS_INSTANCE: &str = r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "properties": {
    "instance_id": {
      "description": "Instance id.",
      "format": "int64",
      "type": "integer"
    }
  },
  "required": [
    "instance_id"
  ],
  "type": "object"
}"##;
    const INPUT_LIST_PROCESS_INSTANCES: &str = r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "properties": {
    "process": {
      "description": "Filter by process name, e.g. \"order_to_cash\".",
      "type": [
        "string",
        "null"
      ]
    },
    "status": {
      "description": "Filter by status: \"active\", \"completed\", or \"cancelled\".",
      "type": [
        "string",
        "null"
      ]
    }
  },
  "type": "object"
}"##;
    const INPUT_LIST_PROCESSES: &str = r##"{
  "properties": {},
  "type": "object"
}"##;
    const INPUT_POST_JOURNAL_ENTRY: &str = r##"{
  "$defs": {
    "LineParam": {
      "description": "A single journal line for a `post_journal_entry` call.",
      "properties": {
        "account_code": {
          "description": "Account code (e.g. \"1000\").",
          "type": "string"
        },
        "credit": {
          "description": "Credit amount in integer cents. Exactly one of debit/credit must be non-zero.",
          "format": "int64",
          "type": "integer"
        },
        "currency": {
          "description": "ISO-4217 currency code (e.g. \"EUR\", \"USD\"). Defaults to \"EUR\" when absent.",
          "type": [
            "string",
            "null"
          ]
        },
        "debit": {
          "description": "Debit amount in integer cents. Exactly one of debit/credit must be non-zero.",
          "format": "int64",
          "type": "integer"
        }
      },
      "required": [
        "account_code",
        "debit",
        "credit"
      ],
      "type": "object"
    }
  },
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "properties": {
    "actor": {
      "description": "Actor identifier recorded in the audit log (e.g. the MCP token subject).",
      "type": "string"
    },
    "effective_date": {
      "description": "Economic date determining the fiscal period (YYYY-MM-DD). Defaults to entry_date when absent.",
      "type": [
        "string",
        "null"
      ]
    },
    "entry_date": {
      "description": "ISO date of the entry (YYYY-MM-DD).",
      "type": "string"
    },
    "idempotency_key": {
      "description": "UUID idempotency key — reuse the same key to replay without double-posting.",
      "format": "uuid",
      "type": "string"
    },
    "journal_code": {
      "description": "Journal code (e.g. \"GJ\" for General Journal).",
      "type": "string"
    },
    "lines": {
      "description": "Entry lines — minimum 2, balanced (Σdebit = Σcredit).",
      "items": {
        "$ref": "#/$defs/LineParam"
      },
      "type": "array"
    },
    "memo": {
      "description": "Optional memo / narrative.",
      "type": [
        "string",
        "null"
      ]
    },
    "reference": {
      "description": "Optional external reference (invoice number, etc.).",
      "type": [
        "string",
        "null"
      ]
    }
  },
  "required": [
    "idempotency_key",
    "journal_code",
    "entry_date",
    "actor",
    "lines"
  ],
  "type": "object"
}"##;
    const INPUT_START_PROCESS: &str = r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "properties": {
    "context": {
      "description": "Optional initial context as a JSON object."
    },
    "idempotency_key": {
      "description": "UUID idempotency key — reuse the same key to replay without creating a\nduplicate instance.",
      "format": "uuid",
      "type": "string"
    },
    "process": {
      "description": "Process name, e.g. \"order_to_cash\".",
      "type": "string"
    },
    "reference": {
      "description": "Optional external reference (order number, customer id, etc.).",
      "type": [
        "string",
        "null"
      ]
    }
  },
  "required": [
    "idempotency_key",
    "process"
  ],
  "type": "object"
}"##;
    const OUTPUT_PROCESS_INSTANCE: &str = r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "properties": {
    "context": true,
    "current_state": {
      "type": "string"
    },
    "id": {
      "format": "int64",
      "type": "integer"
    },
    "process": {
      "type": "string"
    },
    "reference": {
      "type": [
        "string",
        "null"
      ]
    },
    "status": {
      "type": "string"
    }
  },
  "required": [
    "id",
    "process",
    "current_state",
    "status",
    "context"
  ],
  "type": "object"
}"##;
    const OUTPUT_GET_ACCOUNT_BALANCE: &str = r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "properties": {
    "account_code": {
      "type": "string"
    },
    "balance": {
      "description": "Signed balance in integer cents (debit-positive for assets/expenses;\ncredit-positive for liabilities/equity/income).",
      "format": "int64",
      "type": "integer"
    },
    "credits": {
      "description": "Raw credit sum (integer cents).",
      "format": "int64",
      "type": "integer"
    },
    "currency": {
      "description": "ISO-4217 currency code (e.g. \"EUR\").",
      "type": "string"
    },
    "debits": {
      "description": "Raw debit sum (integer cents).",
      "format": "int64",
      "type": "integer"
    }
  },
  "required": [
    "account_code",
    "currency",
    "debits",
    "credits",
    "balance"
  ],
  "type": "object"
}"##;
    const OUTPUT_GET_PROCESS: &str = r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "properties": {
    "mermaid": {
      "description": "Mermaid `stateDiagram-v2` diagram of this process.",
      "type": "string"
    },
    "name": {
      "type": "string"
    },
    "states": {
      "items": {
        "type": "string"
      },
      "type": "array"
    }
  },
  "required": [
    "name",
    "states",
    "mermaid"
  ],
  "type": "object"
}"##;
    const OUTPUT_GET_PROCESS_INSTANCE: &str = r##"{
  "$defs": {
    "AvailableTransitionResult": {
      "description": "One legal next move from the instance's current state.",
      "properties": {
        "capability": {
          "description": "Pass this capability name to `advance_process`.",
          "type": "string"
        },
        "posting": {
          "anyOf": [
            {
              "$ref": "#/$defs/PostingRequirementResult"
            },
            {
              "type": "null"
            }
          ],
          "description": "`Some` when posting amounts are required; `None` for non-posting transitions."
        },
        "to_state": {
          "description": "State the instance will enter when this transition fires.",
          "type": "string"
        }
      },
      "required": [
        "capability",
        "to_state"
      ],
      "type": "object"
    },
    "PostingRequirementResult": {
      "description": "Posting amounts required by an available transition.\n\n`required_amount_keys` lists exactly the keys the caller must supply in the\n`amounts` map. The `credit_roles` field describes every account role this\ntransition credits and is always populated.",
      "properties": {
        "credit_roles": {
          "description": "Every account role this transition credits, always populated.",
          "items": {
            "type": "string"
          },
          "type": "array"
        },
        "debit_role": {
          "description": "The debit role name (informational; the caller uses key `\"amount\"`).",
          "type": "string"
        },
        "required_amount_keys": {
          "description": "Exactly the keys the caller must supply in `amounts`.",
          "items": {
            "type": "string"
          },
          "type": "array"
        }
      },
      "required": [
        "debit_role",
        "credit_roles",
        "required_amount_keys"
      ],
      "type": "object"
    },
    "ProcessInstanceResult": {
      "description": "A process instance snapshot.",
      "properties": {
        "context": true,
        "current_state": {
          "type": "string"
        },
        "id": {
          "format": "int64",
          "type": "integer"
        },
        "process": {
          "type": "string"
        },
        "reference": {
          "type": [
            "string",
            "null"
          ]
        },
        "status": {
          "type": "string"
        }
      },
      "required": [
        "id",
        "process",
        "current_state",
        "status",
        "context"
      ],
      "type": "object"
    },
    "StepLogResult": {
      "description": "One step-log row.",
      "properties": {
        "actor": {
          "type": "string"
        },
        "capability": {
          "type": "string"
        },
        "entry_id": {
          "format": "int64",
          "type": [
            "integer",
            "null"
          ]
        },
        "from_state": {
          "type": "string"
        },
        "id": {
          "format": "int64",
          "type": "integer"
        },
        "to_state": {
          "type": "string"
        }
      },
      "required": [
        "id",
        "from_state",
        "to_state",
        "capability",
        "actor"
      ],
      "type": "object"
    }
  },
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "properties": {
    "available": {
      "description": "Legal next capabilities from the current state.  Empty for terminal states.",
      "items": {
        "$ref": "#/$defs/AvailableTransitionResult"
      },
      "type": "array"
    },
    "instance": {
      "$ref": "#/$defs/ProcessInstanceResult"
    },
    "steps": {
      "items": {
        "$ref": "#/$defs/StepLogResult"
      },
      "type": "array"
    }
  },
  "required": [
    "instance",
    "steps",
    "available"
  ],
  "type": "object"
}"##;
    const OUTPUT_LIST_PROCESS_INSTANCES: &str = r##"{
  "$defs": {
    "ProcessInstanceResult": {
      "description": "A process instance snapshot.",
      "properties": {
        "context": true,
        "current_state": {
          "type": "string"
        },
        "id": {
          "format": "int64",
          "type": "integer"
        },
        "process": {
          "type": "string"
        },
        "reference": {
          "type": [
            "string",
            "null"
          ]
        },
        "status": {
          "type": "string"
        }
      },
      "required": [
        "id",
        "process",
        "current_state",
        "status",
        "context"
      ],
      "type": "object"
    }
  },
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "properties": {
    "instances": {
      "items": {
        "$ref": "#/$defs/ProcessInstanceResult"
      },
      "type": "array"
    }
  },
  "required": [
    "instances"
  ],
  "type": "object"
}"##;
    const OUTPUT_LIST_PROCESSES: &str = r##"{
  "$defs": {
    "ProcessSummary": {
      "description": "A single process entry returned by `list_processes`.",
      "properties": {
        "name": {
          "type": "string"
        },
        "states": {
          "items": {
            "type": "string"
          },
          "type": "array"
        }
      },
      "required": [
        "name",
        "states"
      ],
      "type": "object"
    }
  },
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "properties": {
    "processes": {
      "items": {
        "$ref": "#/$defs/ProcessSummary"
      },
      "type": "array"
    }
  },
  "required": [
    "processes"
  ],
  "type": "object"
}"##;
    const OUTPUT_POST_JOURNAL_ENTRY: &str = r##"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "properties": {
    "balanced": {
      "type": "boolean"
    },
    "entry_id": {
      "format": "int64",
      "type": "integer"
    },
    "replayed": {
      "type": "boolean"
    }
  },
  "required": [
    "entry_id",
    "balanced",
    "replayed"
  ],
  "type": "object"
}"##;

    /// Recursively rewrite a JSON value so every object's keys appear in sorted
    /// order, returning a canonical pretty-printed string. This decouples the
    /// snapshot from any incidental map ordering (`schemars`/`serde_json`
    /// insertion order) so the pinned text only changes when the schema's actual
    /// content changes. Both the live schema and the pinned literal are run
    /// through this before comparison, so the match is on canonical form rather
    /// than on a particular byte serialisation of the same schema.
    fn canonical(v: &Value) -> String {
        fn sort(v: &Value) -> Value {
            match v {
                Value::Object(map) => {
                    let mut keys: Vec<&String> = map.keys().collect();
                    keys.sort();
                    let mut out = serde_json::Map::new();
                    for k in keys {
                        out.insert(k.clone(), sort(map.get(k).unwrap()));
                    }
                    Value::Object(out)
                }
                Value::Array(items) => Value::Array(items.iter().map(sort).collect()),
                other => other.clone(),
            }
        }
        serde_json::to_string_pretty(&sort(v)).unwrap()
    }

    /// GIVEN the tool router,
    /// WHEN the advertised tool contract is serialised,
    /// THEN every tool name and the *complete* JSON of its inputSchema and
    /// outputSchema (types, `required`, nullability, `$defs`, nested shapes --
    /// not just the property-name set) match a pinned snapshot.
    ///
    /// This is the contract an MCP client actually consumes. The name-only test
    /// below cannot see a reshaped schema, so a bump to `rmcp` or to `schemars`
    /// (which generates these schemas) could silently change the public tool
    /// surface -- a renamed nested field, a flipped type (`i64` -> `f64`), a
    /// newly-required property -- while CI stayed green. This test pins the full
    /// schema JSON so any such change fails it. If it fails after a dependency
    /// bump, diff the printed schema and decide deliberately -- do not just
    /// re-pin it.
    #[test]
    fn tool_schemas_match_snapshot() {
        let router = LedgerHandler::tool_router();
        let mut tools = router.list_all();
        tools.sort_by(|a, b| a.name.cmp(&b.name));

        // name -> (canonical input schema JSON, canonical output schema JSON).
        let actual: Vec<(String, String, String)> = tools
            .iter()
            .map(|t| {
                let input = canonical(&Value::Object(t.input_schema.as_ref().clone()));
                let output = t
                    .output_schema
                    .as_ref()
                    .map(|s| canonical(&Value::Object(s.as_ref().clone())))
                    .unwrap_or_default();
                (t.name.to_string(), input, output)
            })
            .collect();

        // The pinned literals are parsed and re-canonicalised, so the snapshot
        // is matched on canonical form -- resilient to incidental serialisation
        // differences but still sensitive to any content change (type,
        // `required`, nullability, `$defs`, nested renames).
        let pin = |s: &str| canonical(&serde_json::from_str::<Value>(s).unwrap());

        let expected: Vec<(&str, String, String)> = vec![
            (
                "advance_process",
                pin(INPUT_ADVANCE_PROCESS),
                pin(OUTPUT_PROCESS_INSTANCE),
            ),
            (
                "get_account_balance",
                pin(INPUT_GET_ACCOUNT_BALANCE),
                pin(OUTPUT_GET_ACCOUNT_BALANCE),
            ),
            (
                "get_process",
                pin(INPUT_GET_PROCESS),
                pin(OUTPUT_GET_PROCESS),
            ),
            (
                "get_process_instance",
                pin(INPUT_GET_PROCESS_INSTANCE),
                pin(OUTPUT_GET_PROCESS_INSTANCE),
            ),
            (
                "list_process_instances",
                pin(INPUT_LIST_PROCESS_INSTANCES),
                pin(OUTPUT_LIST_PROCESS_INSTANCES),
            ),
            (
                "list_processes",
                pin(INPUT_LIST_PROCESSES),
                pin(OUTPUT_LIST_PROCESSES),
            ),
            (
                "post_journal_entry",
                pin(INPUT_POST_JOURNAL_ENTRY),
                pin(OUTPUT_POST_JOURNAL_ENTRY),
            ),
            (
                "start_process",
                pin(INPUT_START_PROCESS),
                pin(OUTPUT_PROCESS_INSTANCE),
            ),
        ];

        // NOTE, surfaced by writing this snapshot: `post_journal_entry` exposes
        // `idempotency_key`, but the two state-changing process tools
        // (`start_process`, `advance_process`) do not. Analysed in #54:
        // `advance_process` is covered incidentally by the illegal-transition
        // guard (its server-derived key changes once the first attempt commits,
        // so the key itself does not dedupe a retry), but `start_process` has no
        // idempotency at all and a retried call creates a duplicate instance.
        // This test only pins the contract as it stands.
        for (a, e) in actual.iter().zip(expected.iter()) {
            assert_eq!(a.0, e.0, "tool name mismatch");
            assert_eq!(
                a.1, e.1,
                "inputSchema for `{}` changed -- an MCP client would see this",
                e.0
            );
            assert_eq!(
                a.2, e.2,
                "outputSchema for `{}` changed -- an MCP client would see this",
                e.0
            );
        }
        assert_eq!(
            actual.len(),
            expected.len(),
            "tool count changed -- an MCP client would see this"
        );
    }

    /// The tool router must list exactly the eight expected tools.
    /// This does not require a live database or MCP client.
    #[test]
    fn tool_router_lists_expected_tools() {
        let router = LedgerHandler::tool_router();
        let tools = router.list_all();
        let names: std::collections::HashSet<&str> =
            tools.iter().map(|t| t.name.as_ref()).collect();

        // Ledger tools
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

        // Engine tools
        assert!(names.contains("start_process"), "missing start_process");
        assert!(names.contains("advance_process"), "missing advance_process");
        assert!(
            names.contains("get_process_instance"),
            "missing get_process_instance"
        );
        assert!(
            names.contains("list_process_instances"),
            "missing list_process_instances"
        );

        assert_eq!(names.len(), 8, "unexpected extra tools: {names:?}");
    }

    /// GIVEN a `PostError::Unbalanced` whose DB message already contains the
    /// `UNBALANCED_ENTRY: ` prefix,
    /// WHEN it is converted to an MCP tool error string,
    /// THEN the prefix is stripped once so the final string is exactly
    /// `UNBALANCED_ENTRY: currency EUR debits 100 != credits 50`.
    #[test]
    fn post_error_unbalanced_strips_double_prefix() {
        let db_message = "UNBALANCED_ENTRY: currency EUR debits 100 != credits 50".to_string();
        let got = post_error_to_string(PostError::Unbalanced {
            message: db_message,
        });
        assert_eq!(
            got,
            "UNBALANCED_ENTRY: currency EUR debits 100 != credits 50"
        );
    }

    /// GIVEN a `PostError::Domain` carrying an `ol_domain::LedgerError`,
    /// WHEN it is converted to an MCP tool error string,
    /// THEN the final string is exactly the single-prefixed domain message.
    #[test]
    fn post_error_domain_strips_double_prefix() {
        let err = ol_domain::LedgerError::Unbalanced {
            currency: "EUR".into(),
            debits: 100,
            credits: 50,
        };
        let got = post_error_to_string(PostError::Domain(err));
        assert_eq!(
            got,
            "UNBALANCED_ENTRY: currency EUR debits 100 != credits 50"
        );
    }

    /// GIVEN a `PostError::AppendOnly` whose DB message already contains the
    /// `APPEND_ONLY: ` prefix,
    /// WHEN it is converted to an MCP tool error string,
    /// THEN the prefix is stripped once so the final string is exactly
    /// `APPEND_ONLY: journal entries are append-only`.
    #[test]
    fn post_error_append_only_strips_double_prefix() {
        let db_message = "APPEND_ONLY: journal entries are append-only".to_string();
        let got = post_error_to_string(PostError::AppendOnly {
            message: db_message,
        });
        assert_eq!(got, "APPEND_ONLY: journal entries are append-only");
    }

    /// GIVEN a `PostError::PeriodClosed` whose DB message already contains the
    /// `PERIOD_CLOSED: ` prefix,
    /// WHEN it is converted to an MCP tool error string,
    /// THEN the prefix is stripped once so the final string is exactly
    /// `PERIOD_CLOSED: fiscal year 2024 is closed`.
    #[test]
    fn post_error_period_closed_strips_double_prefix() {
        let db_message = "PERIOD_CLOSED: fiscal year 2024 is closed".to_string();
        let got = post_error_to_string(PostError::PeriodClosed {
            message: db_message,
        });
        assert_eq!(got, "PERIOD_CLOSED: fiscal year 2024 is closed");
    }
}
