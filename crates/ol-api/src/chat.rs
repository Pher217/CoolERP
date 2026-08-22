//! AI chat endpoint — LLM proposes, deterministic ledger enforces.
//!
//! `POST /chat` runs a local-LLM tool loop: the model picks ledger capabilities
//! and fills their fields; `dispatch_tool` executes them deterministically and
//! returns structured results. The model never computes balances or touches SQL.
//!
//! The loop's control flow IS covered: `tests/chat_max_iterations.rs`,
//! `tests/chat_completed.rs` and `tests/chat_transport_error.rs` drive it against
//! a mock `/api/chat` rather than a live daemon, one test per binary because the
//! handler reads `OLLAMA_URL` from the process environment. What remains
//! uncovered is a real model's *choices* — which tools it picks and how it
//! recovers — not the loop itself. The deterministic `dispatch_tool` executor is
//! unit-tested in `tests/chat_dispatch.rs`.

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use chrono::{Local, NaiveDate};
use ol_domain::Line;
use ol_engine::{
    AdvanceInput, StartInstanceOutcome, advance_instance, get_instance, list_instances,
    start_instance_with_key,
};
use ol_ledger::{PostError, PostRequest, account_balance, post_journal_entry};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

// ─── Timeouts ────────────────────────────────────────────────────────────────

/// Wall-clock ceiling on a single Ollama request, overridable with
/// `OL_CHAT_TIMEOUT_SECS`. Generous enough for a cold cloud model, finite so a
/// hung backend cannot park the handler forever.
const DEFAULT_CHAT_TIMEOUT_SECS: u64 = 120;

/// Maximum tool-loop iterations in one chat turn. Exhausting it truncates the
/// run, which the response reports as `finish_reason: max_iterations`.
const CHAT_MAX_ITERATIONS: usize = 6;

/// Ceiling on establishing the TCP/TLS connection. Short on purpose: an Ollama
/// that is not listening should fail fast rather than consume the full budget.
const CHAT_CONNECT_TIMEOUT_SECS: u64 = 5;

// ─── Public request/response types ───────────────────────────────────────────

/// A single conversation turn (user or assistant).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ChatTurn {
    /// "user" or "assistant"
    pub role: String,
    pub content: String,
}

/// Request body for `POST /chat`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ChatRequest {
    pub message: String,
    /// Conversation history for multi-turn context.
    #[serde(default)]
    pub history: Vec<ChatTurn>,
    /// Stable identifier for this conversation, supplied by the client and kept
    /// for the conversation's lifetime.
    ///
    /// It exists so tool idempotency keys can be derived rather than minted at
    /// random (#86). When absent a fresh one is generated per request, which
    /// still deduplicates a model's retries *within* this request — the live
    /// double-post path — but cannot deduplicate across requests. There is no
    /// server-side conversation store yet, so this is the only identity available.
    #[serde(default)]
    pub conversation_id: Option<Uuid>,
}

/// A single tool invocation with its result.
#[derive(Debug, Serialize, ToSchema)]
pub struct ChatAction {
    pub tool: String,
    pub args: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

/// View directive returned to the client so the UI can switch modules/focus.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ViewDirective {
    pub module: String,
    pub focus: Option<String>,
}

/// Response body for `POST /chat`.
#[derive(Debug, Serialize, ToSchema)]
pub struct ChatResponse {
    pub reply: String,
    pub actions: Vec<ChatAction>,
    #[serde(default)]
    pub view: Option<ViewDirective>,
    /// Why the tool loop stopped. Anything other than `completed` means the run
    /// did NOT finish, and `reply` says so rather than claiming success (#88).
    pub finish_reason: FinishReason,
}

/// Why a chat run ended.
///
/// Before this existed, a run truncated by the iteration cap was reported with
/// the canned text "I completed the requested operations." and a `200` — telling
/// a user that an abandoned multi-step posting had succeeded. For an ERP that is
/// the wrong failure direction: a user who believes an invoice was posted and
/// moves on is worse off than one who sees an error (#88).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum FinishReason {
    /// The model returned a final text answer.
    #[default]
    Completed,
    /// The iteration cap was exhausted while the model was still calling tools.
    MaxIterations,
    /// The model's response could not be parsed.
    ModelError,
    /// Ollama could not be reached.
    TransportError,
}

// ─── Tool executor ────────────────────────────────────────────────────────────

/// Namespace for chat-derived idempotency keys. A fixed v5 namespace UUID —
/// changing it re-keys every future derivation, so it must stay stable.
const CHAT_IDEMPOTENCY_NAMESPACE: Uuid = Uuid::from_bytes([
    0x6f, 0x2a, 0x1c, 0x4e, 0x9b, 0x3d, 0x47, 0x8a, 0xa1, 0x52, 0xc8, 0x0d, 0x33, 0x91, 0x7e, 0x64,
]);

/// Identity of the turn a tool call belongs to, used to derive idempotency keys.
///
/// The chat surface used to mint `Uuid::new_v4()` per post, so `UNIQUE (operation,
/// idempotency_key)` could never engage and the AI path was the one path that
/// could double-post (#86). Deriving the key instead makes a retry re-derive the
/// same key, which the ledger then dedupes exactly as it does for REST and MCP.
#[derive(Debug, Clone, Copy)]
pub struct ToolContext {
    /// Identifies the conversation. Client-supplied so it is stable across the
    /// turns of one conversation; a fresh one per request when absent, which
    /// degrades to "dedupe within this request only".
    pub conversation_id: Uuid,
    /// Which turn of the conversation this is. `history.len()` — it advances once
    /// per user message, NOT once per tool-loop iteration.
    pub turn: usize,
}

impl ToolContext {
    /// Derive the idempotency key for one tool call.
    ///
    /// The three components are each load-bearing, and the tests below pin why:
    ///
    /// * `conversation_id` — separates two users doing the same thing.
    /// * `turn` — lets a user deliberately repeat a post in a later turn. It is
    ///   the TURN index, not the loop iteration: a model that re-issues the same
    ///   call on a later iteration of the *same* turn (the live double-post path
    ///   in #86, where it misreads a success as a failure) must re-derive the
    ///   SAME key and dedupe.
    /// * canonical args — two genuinely different posts in one turn must not
    ///   collapse into one. `serde_json::Map` is a `BTreeMap` here (the
    ///   `preserve_order` feature is off), so serialisation is key-sorted and
    ///   stable.
    pub fn idempotency_key(&self, tool: &str, args: &serde_json::Value) -> Uuid {
        let material = format!("{}|{}|{}|{}", self.conversation_id, self.turn, tool, args);
        Uuid::new_v5(&CHAT_IDEMPOTENCY_NAMESPACE, material.as_bytes())
    }
}

/// Execute a named ledger tool deterministically.
///
/// The LLM proposes a tool call; this function enforces every invariant by
/// delegating to `ol_ledger` and `ol_process`. The model never computes
/// balances or touches SQL.
pub async fn dispatch_tool(
    pool: &PgPool,
    name: &str,
    args: &serde_json::Value,
    ctx: &ToolContext,
) -> Result<serde_json::Value, String> {
    match name {
        "get_account_balance" => {
            let account_code = args["account_code"]
                .as_str()
                .ok_or_else(|| "missing account_code".to_string())?;

            match account_balance(pool, account_code).await {
                Ok(bal) => {
                    serde_json::to_value(&bal).map_err(|e| format!("serialization error: {e}"))
                }
                Err(PostError::AccountNotFound(code)) => Err(format!("account not found: {code}")),
                Err(e) => Err(e.to_string()),
            }
        }

        "list_processes" => {
            let dir = processes_dir();
            let read_dir = std::fs::read_dir(&dir)
                .map_err(|e| format!("failed to read processes directory: {e}"))?;

            let mut names: Vec<String> = read_dir
                .flatten()
                .filter_map(|entry| {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
                        path.file_stem()
                            .and_then(|s| s.to_str())
                            .map(ToOwned::to_owned)
                    } else {
                        None
                    }
                })
                .collect();
            names.sort();

            serde_json::to_value(names).map_err(|e| format!("serialization error: {e}"))
        }

        "post_journal_entry" => {
            let journal_code = args["journal_code"]
                .as_str()
                .ok_or_else(|| "missing journal_code".to_string())?
                .to_string();

            let entry_date_str = args["entry_date"]
                .as_str()
                .ok_or_else(|| "missing entry_date".to_string())?;
            let entry_date = NaiveDate::parse_from_str(entry_date_str, "%Y-%m-%d")
                .map_err(|e| format!("invalid entry_date: {e}"))?;

            let memo = args["memo"].as_str().map(ToOwned::to_owned);

            let lines_val = args["lines"]
                .as_array()
                .ok_or_else(|| "missing lines array".to_string())?;

            let mut lines = Vec::with_capacity(lines_val.len());
            for lv in lines_val {
                let account_code = lv["account_code"]
                    .as_str()
                    .ok_or_else(|| "line missing account_code".to_string())?
                    .to_string();
                let debit = lv["debit"]
                    .as_i64()
                    .ok_or_else(|| "line missing debit (integer cents)".to_string())?;
                let credit = lv["credit"]
                    .as_i64()
                    .ok_or_else(|| "line missing credit (integer cents)".to_string())?;
                let currency = lv["currency"].as_str().unwrap_or("EUR").to_string();
                lines.push(Line::in_currency(account_code, debit, credit, currency));
            }

            let req = PostRequest {
                idempotency_key: ctx.idempotency_key("post_journal_entry", args),
                journal_code,
                entry_date,
                effective_date: None,
                memo,
                reference: None,
                actor: "ai-chat".to_string(),
                lines,
            };

            match post_journal_entry(pool, &req).await {
                Ok(result) => {
                    let v = serde_json::json!({
                        "entry_id": result.entry_id,
                        "balanced": result.balanced,
                        "replayed": result.replayed,
                    });
                    Ok(v)
                }
                Err(e) => Err(e.to_string()),
            }
        }

        "show_view" => {
            let module = args["module"]
                .as_str()
                .ok_or_else(|| "missing module".to_string())?;
            if !matches!(module, "ledger" | "inventory" | "workflows") {
                return Err(format!("invalid module: {module}"));
            }
            let focus = args["focus"].as_str().map(ToOwned::to_owned);
            Ok(serde_json::json!({
                "ok": true,
                "module": module,
                "focus": focus,
            }))
        }

        "list_inventory" => {
            let rows = crate::inventory::list_inventory_rows(pool)
                .await
                .map_err(|e| format!("database error: {e}"))?;
            serde_json::to_value(rows).map_err(|e| format!("serialization error: {e}"))
        }

        "receive_stock" => {
            let sku = args["sku"]
                .as_str()
                .ok_or_else(|| "missing sku".to_string())?
                .to_string();
            let location_code = args["location_code"]
                .as_str()
                .ok_or_else(|| "missing location_code".to_string())?
                .to_string();
            let qty = args["qty"]
                .as_str()
                .ok_or_else(|| "missing qty".to_string())?
                .to_string();
            let unit_cost = args["unit_cost"].as_str().map(ToOwned::to_owned);

            let resp = crate::inventory::receive_stock_core(
                pool,
                crate::inventory::ReceiveStockRequest {
                    idempotency_key: ctx.idempotency_key("receive_stock", args),
                    sku,
                    location_code,
                    qty,
                    unit_cost,
                    actor: "ai-chat".to_string(),
                },
            )
            .await?;

            serde_json::to_value(resp).map_err(|e| format!("serialization error: {e}"))
        }

        "start_process" => {
            let process = args["process"]
                .as_str()
                .ok_or_else(|| "missing process".to_string())?
                .to_string();
            let reference = args["reference"].as_str().map(ToOwned::to_owned);
            let context = args["context"].clone();
            let context = if context.is_null() {
                serde_json::Value::Object(Default::default())
            } else {
                context
            };

            let StartInstanceOutcome { instance: inst, .. } = start_instance_with_key(
                pool,
                &process,
                reference,
                context,
                ctx.idempotency_key("start_process", args),
            )
            .await
            .map_err(|e| e.to_string())?;

            serde_json::to_value(serde_json::json!({
                "id": inst.id,
                "process": inst.process,
                "current_state": inst.current_state,
                "status": inst.status,
                "reference": inst.reference,
                "context": inst.context,
            }))
            .map_err(|e| format!("serialization error: {e}"))
        }

        "advance_process" => {
            let instance_id = args["instance_id"]
                .as_i64()
                .ok_or_else(|| "missing instance_id (integer)".to_string())?;
            let capability = args["capability"]
                .as_str()
                .ok_or_else(|| "missing capability".to_string())?
                .to_string();

            let amounts: std::collections::HashMap<String, i64> = args["amounts"]
                .as_object()
                .map(|obj| {
                    obj.iter()
                        .filter_map(|(k, v)| v.as_i64().map(|n| (k.clone(), n)))
                        .collect()
                })
                .unwrap_or_default();

            let context_patch = args["context_patch"].clone();
            let context_patch = if context_patch.is_null() {
                serde_json::Value::Object(Default::default())
            } else {
                context_patch
            };

            let entry_date = Local::now().date_naive();

            let input = AdvanceInput {
                amounts,
                context_patch,
                entry_date,
            };

            let inst = advance_instance(pool, instance_id, &capability, "ai-chat", input)
                .await
                .map_err(|e| e.to_string())?;

            serde_json::to_value(serde_json::json!({
                "id": inst.id,
                "process": inst.process,
                "current_state": inst.current_state,
                "status": inst.status,
                "reference": inst.reference,
                "context": inst.context,
            }))
            .map_err(|e| format!("serialization error: {e}"))
        }

        "get_process_instance" => {
            let instance_id = args["instance_id"]
                .as_i64()
                .ok_or_else(|| "missing instance_id (integer)".to_string())?;

            let result = get_instance(pool, instance_id)
                .await
                .map_err(|e| e.to_string())?;

            let steps_val: Vec<serde_json::Value> = result
                .steps
                .into_iter()
                .map(|s| {
                    serde_json::json!({
                        "id": s.id,
                        "from_state": s.from_state,
                        "to_state": s.to_state,
                        "capability": s.capability,
                        "actor": s.actor,
                        "entry_id": s.entry_id,
                    })
                })
                .collect();

            // Serialize available transitions so the LLM knows the legal next moves.
            let available_val: Vec<serde_json::Value> = result
                .available
                .into_iter()
                .map(|at| {
                    serde_json::json!({
                        "capability": at.capability,
                        "to_state": at.to_state,
                        "posting": at.posting.map(|p| serde_json::json!({
                            "debit_role": p.debit_role,
                            "credit_roles": p.credit_roles,
                            "required_amount_keys": p.required_amount_keys,
                        })),
                    })
                })
                .collect();

            let inst = result.instance;
            serde_json::to_value(serde_json::json!({
                "id": inst.id,
                "process": inst.process,
                "current_state": inst.current_state,
                "status": inst.status,
                "reference": inst.reference,
                "context": inst.context,
                "steps": steps_val,
                "available": available_val,
            }))
            .map_err(|e| format!("serialization error: {e}"))
        }

        "list_process_instances" => {
            let process = args["process"].as_str();
            let status = args["status"].as_str();

            let list = list_instances(pool, process, status)
                .await
                .map_err(|e| e.to_string())?;

            let vals: Vec<serde_json::Value> = list
                .into_iter()
                .map(|inst| {
                    serde_json::json!({
                        "id": inst.id,
                        "process": inst.process,
                        "current_state": inst.current_state,
                        "status": inst.status,
                        "reference": inst.reference,
                    })
                })
                .collect();

            serde_json::to_value(vals).map_err(|e| format!("serialization error: {e}"))
        }

        unknown => Err(format!("unknown tool: {unknown}")),
    }
}

// ─── Tools schema (OpenAI / Ollama function format) ───────────────────────────

fn tools() -> serde_json::Value {
    serde_json::json!([
        {
            "type": "function",
            "function": {
                "name": "get_account_balance",
                "description": "Return the current balance of a ledger account. \
                                Always call this to look up balances — never estimate them.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "account_code": {
                            "type": "string",
                            "description": "The account code, e.g. \"1000\""
                        }
                    },
                    "required": ["account_code"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "list_processes",
                "description": "List the names of all available accounting workflow processes.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "post_journal_entry",
                "description": "Record a double-entry journal entry. \
                                Lines MUST balance per currency (sum of debits == sum of credits \
                                within each currency). Amounts are integer cents (100 = €1.00). \
                                The ledger will reject any unbalanced entry.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "journal_code": {
                            "type": "string",
                            "description": "Journal code, e.g. \"GEN\""
                        },
                        "entry_date": {
                            "type": "string",
                            "description": "ISO 8601 date, e.g. \"2025-01-15\""
                        },
                        "memo": {
                            "type": "string",
                            "description": "Optional description of the entry"
                        },
                        "lines": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "account_code": { "type": "string" },
                                    "debit":  { "type": "integer", "description": "Integer cents" },
                                    "credit": { "type": "integer", "description": "Integer cents" },
                                    "currency": { "type": "string", "description": "ISO-4217, defaults to EUR" }
                                },
                                "required": ["account_code", "debit", "credit"]
                            }
                        }
                    },
                    "required": ["journal_code", "entry_date", "lines"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "show_view",
                "description": "Switch the on-screen view to a different module. \
                                Use this when the user asks to open ledger, inventory, or workflows.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "module": {
                            "type": "string",
                            "enum": ["ledger", "inventory", "workflows"],
                            "description": "Module to show"
                        },
                        "focus": {
                            "type": "string",
                            "description": "Optional focus within the module, e.g. an account code or SKU"
                        }
                    },
                    "required": ["module"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "list_inventory",
                "description": "List all inventory items with their current on-hand quantities.",
                "parameters": {
                    "type": "object",
                    "properties": {}
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "receive_stock",
                "description": "Receive stock into a location. \
                                Quantity is a decimal string parsed by the database as NUMERIC.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "sku": {
                            "type": "string",
                            "description": "Item SKU, e.g. \"WIDGET-A\""
                        },
                        "location_code": {
                            "type": "string",
                            "description": "Location code, e.g. \"MAIN\""
                        },
                        "qty": {
                            "type": "string",
                            "description": "Quantity as a decimal string, e.g. \"12.5000\""
                        },
                        "unit_cost": {
                            "type": "string",
                            "description": "Optional unit cost as a decimal string"
                        }
                    },
                    "required": ["sku", "location_code", "qty"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "start_process",
                "description": "Start a new instance of a business process (e.g. order_to_cash, purchase_to_pay). \
                                Returns the new instance with its id and current state. \
                                After starting, call advance_process to step through transitions.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "process": {
                            "type": "string",
                            "description": "Process name, e.g. \"order_to_cash\""
                        },
                        "reference": {
                            "type": "string",
                            "description": "Optional external reference, e.g. an order number"
                        },
                        "context": {
                            "type": "object",
                            "description": "Optional initial context JSON"
                        }
                    },
                    "required": ["process"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "advance_process",
                "description": "Advance a process instance by one transition. \
                                Pick a capability from the `available` list returned by get_process_instance \
                                and call this tool with that exact capability name. \
                                For transitions with a posting requirement (`available[n].posting != null`), \
                                supply `amounts` as a map keyed by EXACTLY the entries in \
                                `posting.required_amount_keys` — 'amount' is always the debit total in \
                                integer cents (e.g. {\"amount\": 10000} for a single-credit step, whose \
                                one credited account takes the whole total, or \
                                {\"amount\": 11000, \"sales_revenue\": 10000, \"tax_payable\": 1000} \
                                for a multi-credit step, where the credit values must sum to 'amount'). \
                                `posting.credit_roles` is descriptive — it names the accounts that get \
                                credited, and is NOT the list of keys to send. \
                                The ledger enforces double-entry balance — only legal transitions are accepted. \
                                On error, read the message: it lists the available capabilities.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "instance_id": {
                            "type": "integer",
                            "description": "Instance id returned by start_process"
                        },
                        "capability": {
                            "type": "string",
                            "description": "Transition capability, e.g. \"confirm_order\""
                        },
                        "amounts": {
                            "type": "object",
                            "description": "Integer-cents amounts for posting steps. \
                                            Include key 'amount' = the debit total, \
                                            and for a multi-credit posting one key per credit account role \
                                            (e.g. sales_revenue, tax_payable) whose values sum to 'amount'. \
                                            Non-posting steps need no amounts."
                        },
                        "context_patch": {
                            "type": "object",
                            "description": "JSON object merged into the instance context"
                        }
                    },
                    "required": ["instance_id", "capability"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "list_process_instances",
                "description": "List running or completed process instances. \
                                Optionally filter by process name and/or status (active, completed, cancelled).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "process": {
                            "type": "string",
                            "description": "Filter by process name, e.g. \"order_to_cash\""
                        },
                        "status": {
                            "type": "string",
                            "enum": ["active", "completed", "cancelled"],
                            "description": "Filter by status"
                        }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "get_process_instance",
                "description": "Get a process instance, its full step audit log, and the `available` \
                                next capabilities the instance can accept right now. \
                                The `available` array tells you exactly which capability names are legal \
                                from the current state, and (for posting transitions) which amounts you \
                                must supply. Always call this before advance_process so you pick the \
                                correct capability — never guess capability names.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "instance_id": {
                            "type": "integer",
                            "description": "Instance id"
                        }
                    },
                    "required": ["instance_id"]
                }
            }
        }
    ])
}

// ─── System prompt ────────────────────────────────────────────────────────────

const SYSTEM_PROMPT: &str = "You are the CoolERP assistant. You operate a double-entry \
accounting ledger and business-process engine by calling tools. \
\n\nLEDGER: NEVER compute balances yourself — call get_account_balance. To record money \
movement, call post_journal_entry with lines whose debits equal credits within each currency \
(amounts are integer cents: 100 = 1.00). \
\n\nPROCESSES: You can run structured business processes (e.g. order_to_cash, purchase_to_pay). \
Use start_process to create an instance. To step through a process: \
1. Call get_process_instance to see the `available` array — it lists every legal next capability \
   for the current state, with posting requirements when amounts are needed. \
2. Pick one capability from `available` and call advance_process with that EXACT capability name. \
   NEVER guess capability names — always read them from `available`. \
3. For a posting step (`available[n].posting != null`), supply `amounts` keyed by exactly the \
   entries in `posting.required_amount_keys`: 'amount' = the debit total, plus one key per credited \
   role when there is more than one (those must sum to 'amount'). `posting.credit_roles` names the \
   accounts being credited — it is descriptive, not the list of keys to send. \
If advance_process returns an error, read it — it names the available capabilities from the \
current state. NEVER tell the user a step succeeded unless the tool returned success. \
Use list_process_instances and get_process_instance to inspect running or completed instances. \
The engine enforces only legal transitions; the ledger enforces double-entry balance. \
\n\nINVENTORY: read or operate inventory with list_inventory and receive_stock. \
UI: switch the on-screen view with show_view (modules: ledger, inventory, workflows). \
If a tool returns an error, read it and explain it plainly. Be concise.";

// ─── Ollama structs ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct OllamaRequest<'a> {
    model: &'a str,
    messages: &'a [serde_json::Value],
    tools: serde_json::Value,
    stream: bool,
}

#[derive(Debug, Deserialize)]
struct OllamaResponse {
    message: OllamaMessage,
}

#[derive(Debug, Deserialize)]
struct OllamaMessage {
    #[allow(dead_code)]
    role: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    tool_calls: Vec<OllamaToolCall>,
}

#[derive(Debug, Deserialize)]
struct OllamaToolCall {
    function: OllamaToolFunction,
}

#[derive(Debug, Deserialize)]
struct OllamaToolFunction {
    name: String,
    /// Arguments may arrive as a JSON object or as a JSON-encoded string.
    arguments: serde_json::Value,
}

// ─── POST /chat handler ───────────────────────────────────────────────────────

/// Chat with the CoolERP AI assistant.
///
/// The assistant uses tool calls to query and update the ledger.
/// The model never computes balances or touches SQL directly — all ledger
/// operations go through the deterministic `dispatch_tool` executor.
#[utoipa::path(
    post,
    path = "/chat",
    request_body = ChatRequest,
    responses(
        (status = 200, description = "Assistant reply with any tool actions taken. Check `finish_reason`: `max_iterations` means the run was truncated and did NOT finish", body = ChatResponse),
        (status = 502, description = "The model returned a response that could not be parsed", body = ChatResponse),
        (status = 503, description = "LLM backend unreachable", body = ChatResponse),
    )
)]
pub async fn chat(State(pool): State<PgPool>, Json(req): Json<ChatRequest>) -> impl IntoResponse {
    let model_name =
        std::env::var("OL_CHAT_MODEL").unwrap_or_else(|_| "kimi-k2.7-code:cloud".to_string());
    let ollama_url =
        std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://localhost:11434".to_string());

    // Build initial message list.
    let mut messages: Vec<serde_json::Value> = Vec::new();
    messages.push(serde_json::json!({ "role": "system", "content": SYSTEM_PROMPT }));
    for turn in &req.history {
        messages.push(serde_json::json!({ "role": turn.role, "content": turn.content }));
    }
    messages.push(serde_json::json!({ "role": "user", "content": req.message }));

    // A chat turn can legitimately take a while (a cloud-hosted model, a cold
    // load), but it must not take forever: without a timeout a hung Ollama
    // connection parks this handler — and its database pool slot — indefinitely.
    // Bounded per request, not per handler, so the 6-iteration loop cannot
    // silently multiply into an unbounded wait.
    let timeout_secs: u64 = std::env::var("OL_CHAT_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_CHAT_TIMEOUT_SECS);
    let client = match reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(CHAT_CONNECT_TIMEOUT_SECS))
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ChatResponse {
                    reply: format!("Could not construct the HTTP client: {e}"),
                    actions: Vec::new(),
                    view: None,
                    finish_reason: FinishReason::TransportError,
                }),
            )
                .into_response();
        }
    };
    // One identity per request. `turn` is history.len() so it advances per user
    // message, not per loop iteration — see ToolContext::idempotency_key.
    let tool_ctx = ToolContext {
        conversation_id: req.conversation_id.unwrap_or_else(Uuid::new_v4),
        turn: req.history.len(),
    };
    let mut actions: Vec<ChatAction> = Vec::new();
    let mut reply = String::new();
    let mut view: Option<ViewDirective> = None;
    // Only a final text answer from the model counts as completion. If the loop
    // falls out of the bottom, the cap truncated a run that was still working.
    let mut finish_reason = FinishReason::MaxIterations;

    // Tool loop — max CHAT_MAX_ITERATIONS iterations.
    'outer: for _ in 0..CHAT_MAX_ITERATIONS {
        let ollama_req = OllamaRequest {
            model: &model_name,
            messages: &messages,
            tools: tools(),
            stream: false,
        };

        let resp = match client
            .post(format!("{ollama_url}/api/chat"))
            .json(&ollama_req)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                reply = format!(
                    "The AI assistant is currently offline (could not reach Ollama: {e}). \
                     Please start Ollama and try again."
                );
                // 503, not 200: the OpenAPI doc has always advertised this variant
                // while the handler returned OK on every path (#88). Reconciled by
                // making the behaviour match the published contract.
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    Json(ChatResponse {
                        reply,
                        actions,
                        view,
                        finish_reason: FinishReason::TransportError,
                    }),
                )
                    .into_response();
            }
        };

        let ollama_resp: OllamaResponse = match resp.json().await {
            Ok(r) => r,
            Err(e) => {
                reply = format!(
                    "The AI assistant returned an unexpected response: {e}. \
                     Check that the model is loaded in Ollama."
                );
                return (
                    StatusCode::BAD_GATEWAY,
                    Json(ChatResponse {
                        reply,
                        actions,
                        view,
                        finish_reason: FinishReason::ModelError,
                    }),
                )
                    .into_response();
            }
        };

        let msg = ollama_resp.message;

        if msg.tool_calls.is_empty() {
            // Final text reply — done.
            reply = msg.content;
            finish_reason = FinishReason::Completed;
            break 'outer;
        }

        // Append the assistant message (with tool_calls) to history.
        // Some models emit content alongside tool_calls; preserve it.
        let assistant_msg = if msg.content.is_empty() {
            serde_json::json!({
                "role": "assistant",
                "content": null,
                "tool_calls": msg.tool_calls.iter().map(|tc| serde_json::json!({
                    "function": {
                        "name": tc.function.name,
                        "arguments": tc.function.arguments,
                    }
                })).collect::<Vec<_>>()
            })
        } else {
            serde_json::json!({
                "role": "assistant",
                "content": msg.content,
                "tool_calls": msg.tool_calls.iter().map(|tc| serde_json::json!({
                    "function": {
                        "name": tc.function.name,
                        "arguments": tc.function.arguments,
                    }
                })).collect::<Vec<_>>()
            })
        };
        messages.push(assistant_msg);

        // Execute each tool call and append result messages.
        for tc in &msg.tool_calls {
            // Arguments may arrive as an object or as a JSON-encoded string.
            let args: serde_json::Value = match &tc.function.arguments {
                serde_json::Value::String(s) => {
                    serde_json::from_str(s).unwrap_or_else(|_| serde_json::json!({}))
                }
                other => other.clone(),
            };

            let tool_result = dispatch_tool(&pool, &tc.function.name, &args, &tool_ctx).await;

            let (result_val, error_str, tool_content) = match &tool_result {
                Ok(v) => (Some(v.clone()), None, v.to_string()),
                Err(e) => (None, Some(e.clone()), format!("ERROR: {e}")),
            };

            // A show_view call also drives the client-side view directive.
            if tc.function.name == "show_view" {
                if let Some(module) = args["module"].as_str() {
                    view = Some(ViewDirective {
                        module: module.to_string(),
                        focus: args["focus"].as_str().map(ToOwned::to_owned),
                    });
                }
            }

            actions.push(ChatAction {
                tool: tc.function.name.clone(),
                args: args.clone(),
                result: result_val,
                error: error_str,
            });

            messages.push(serde_json::json!({
                "role": "tool",
                "content": tool_content,
            }));
        }
    }

    if reply.is_empty() {
        // Never claim completion here. This branch is reached when the model
        // produced no closing text — most often because the iteration cap cut a
        // run short mid-sequence (#88). Say which, and say what did land, rather
        // than asserting success for work that may have been abandoned.
        reply = match finish_reason {
            FinishReason::Completed => "The assistant finished without a closing message. \
                 Check the actions list for what was done."
                .to_string(),
            _ => format!(
                "I ran out of steps after {CHAT_MAX_ITERATIONS} tool calls and stopped before \
                 finishing. {} action(s) were carried out and are listed below — review them \
                 before retrying, because re-asking will run them again.",
                actions.len()
            ),
        };
    }

    // A truncated run is not a success. It is not a server fault either — the
    // work that did land is real and reported — so it is 200 with an explicit
    // finish_reason rather than an error status that would imply nothing happened.
    (
        StatusCode::OK,
        Json(ChatResponse {
            reply,
            actions,
            view,
            finish_reason,
        }),
    )
        .into_response()
}

// ─── Process dir helper (shared with lib.rs) ──────────────────────────────────

fn processes_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("OL_PROCESSES_DIR") {
        return std::path::PathBuf::from(dir);
    }
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("processes")
}

// ─── Cross-surface tool-drift tests ──────────────────────────────────────────

/// CoolERP exposes two agent-facing tool surfaces that are written by hand in
/// separate crates with no shared definition: the chat manifest in `tools()`
/// above, and the MCP `#[tool]` router in `ol-mcp`. `ol-api` does not depend on
/// `ol-mcp` at runtime, so nothing forces them to agree and they have already
/// diverged (issue #89).
///
/// These tests pin the divergence rather than merely reporting it. Two lists do
/// the work, and the distinction between them is the point:
///
/// * `INTENTIONAL_*` — differences that are correct and permanent.
/// * `KNOWN_DRIFT_*` — differences that are *defects*, each citing the issue
///   that tracks it. These are asserted as an **exact** set, so the test fails
///   both when new drift appears and when tracked drift is fixed without being
///   removed from the list. It stops the bleeding without pretending the wound
///   is a feature.
#[cfg(test)]
mod tool_surface_tests {
    use super::tools;
    use std::collections::{BTreeMap, BTreeSet};

    /// Correct, permanent chat-only tools.
    const INTENTIONAL_CHAT_ONLY: &[(&str, &str)] = &[(
        "show_view",
        "UI directive: switches the on-screen module in the web SPA. It drives \
         the browser, not the ledger, and is meaningless to a headless MCP client.",
    )];

    /// Correct, permanent MCP-only tools.
    const INTENTIONAL_MCP_ONLY: &[(&str, &str)] = &[];

    /// Tool-name drift that is a defect, not a decision. Each entry cites its
    /// tracking issue. Removing an entry is how a fix is landed.
    const KNOWN_DRIFT_NAMES: &[(&str, &str)] = &[
        (
            "list_inventory",
            "chat-only; unfiled drift — inventory reads are absent from MCP for no \
             stated reason. Converge under #89.",
        ),
        (
            "receive_stock",
            "chat-only; must NOT be promoted to MCP until #90 gives it an \
             idempotency contract — canonising an unsafe write is worse than the drift.",
        ),
        (
            "get_process",
            "MCP-only; the chat agent cannot fetch a process definition. Converge under #89.",
        ),
    ];

    /// Required-parameter drift on tools present in both surfaces.
    /// `(tool, param, why)` — `param` is required on one surface only.
    const KNOWN_DRIFT_PARAMS: &[(&str, &str, &str)] = &[
        (
            "post_journal_entry",
            "idempotency_key",
            "INTENTIONAL since the #86 fix: the chat schema deliberately does not \
             expose this field, because the key is DERIVED server-side from \
             (conversation, turn, tool, canonical args) via ToolContext. Exposing \
             it would make deduplication depend on model behaviour, which is the \
             thing #86 was about. MCP and REST callers are real clients and supply \
             their own; the model is not a client in that sense.",
        ),
        (
            "post_journal_entry",
            "actor",
            "Required on MCP, absent from the chat schema, which hardcodes \
             actor=\"ai-chat\". Audit attribution differs by surface (#89).",
        ),
        (
            "start_process",
            "idempotency_key",
            "INTENTIONAL since the #54 fix: the chat schema deliberately does not \
             expose this field, because the key is DERIVED server-side from \
             (conversation, turn, tool, canonical args) via ToolContext. Exposing \
             it would make deduplication depend on model behaviour. MCP and REST \
             callers are real clients and supply their own; the model is not a \
             client in that sense.",
        ),
    ];

    fn chat_tools() -> Vec<serde_json::Value> {
        tools()
            .as_array()
            .expect("chat tools() must be a JSON array")
            .clone()
    }

    fn chat_tool_names() -> BTreeSet<String> {
        chat_tools()
            .iter()
            .map(|t| {
                t["function"]["name"]
                    .as_str()
                    .expect("every chat tool needs a function.name")
                    .to_string()
            })
            .collect()
    }

    /// name -> required-parameter names, chat surface.
    fn chat_required() -> BTreeMap<String, BTreeSet<String>> {
        chat_tools()
            .iter()
            .map(|t| {
                let f = &t["function"];
                let req = f["parameters"]["required"]
                    .as_array()
                    .map(|a| a.iter().map(|v| v.as_str().unwrap().to_string()).collect())
                    .unwrap_or_default();
                (f["name"].as_str().unwrap().to_string(), req)
            })
            .collect()
    }

    fn mcp_tool_names() -> BTreeSet<String> {
        ol_mcp::advertised_tools()
            .iter()
            .map(|t| t.name.to_string())
            .collect()
    }

    /// name -> required-parameter names, MCP surface.
    fn mcp_required() -> BTreeMap<String, BTreeSet<String>> {
        ol_mcp::advertised_tools()
            .iter()
            .map(|t| {
                let req = t
                    .input_schema
                    .get("required")
                    .and_then(|r| r.as_array())
                    .map(|a| a.iter().map(|v| v.as_str().unwrap().to_string()).collect())
                    .unwrap_or_default();
                (t.name.to_string(), req)
            })
            .collect()
    }

    /// GIVEN the chat tool manifest and the MCP tool router,
    /// WHEN the two tool-name sets are compared,
    /// THEN the difference is exactly the intentional exceptions plus the
    /// known-drift list — no more, and no less.
    #[test]
    fn tool_name_drift_is_exactly_the_documented_set() {
        let chat = chat_tool_names();
        let mcp = mcp_tool_names();

        let mut actual_divergent: BTreeSet<String> = chat.difference(&mcp).cloned().collect();
        actual_divergent.extend(mcp.difference(&chat).cloned());

        let intentional: BTreeSet<String> = INTENTIONAL_CHAT_ONLY
            .iter()
            .chain(INTENTIONAL_MCP_ONLY)
            .map(|(n, _)| n.to_string())
            .collect();
        let known_drift: BTreeSet<String> = KNOWN_DRIFT_NAMES
            .iter()
            .map(|(n, _)| n.to_string())
            .collect();
        let expected: BTreeSet<String> = intentional.union(&known_drift).cloned().collect();

        let unexpected: Vec<&String> = actual_divergent.difference(&expected).collect();
        let stale: Vec<&String> = expected.difference(&actual_divergent).collect();

        assert!(
            unexpected.is_empty(),
            "NEW tool-surface drift: {unexpected:?}\n\
             A tool was added to one agent surface and not the other. Add it to the \
             other surface, or document it in INTENTIONAL_* / KNOWN_DRIFT_NAMES with a reason."
        );
        assert!(
            stale.is_empty(),
            "documented drift no longer exists: {stale:?}\n\
             The surfaces now agree on these — delete their entries from \
             KNOWN_DRIFT_NAMES / INTENTIONAL_* so the list keeps telling the truth."
        );
    }

    /// GIVEN a tool present on both surfaces,
    /// WHEN its required parameters are compared,
    /// THEN the sets differ only by the entries in `KNOWN_DRIFT_PARAMS`.
    ///
    /// This is the field-level half. It is the check that would have caught
    /// issue #86 — `post_journal_entry` requires `idempotency_key` on MCP and
    /// REST but has no such field on chat, so that path cannot deduplicate.
    #[test]
    fn required_parameter_drift_is_exactly_the_documented_set() {
        let chat = chat_required();
        let mcp = mcp_required();

        let mut actual: BTreeSet<(String, String)> = BTreeSet::new();
        for (name, chat_req) in &chat {
            let Some(mcp_req) = mcp.get(name) else {
                continue; // name-set divergence is the other test's job
            };
            for p in mcp_req.symmetric_difference(chat_req) {
                actual.insert((name.clone(), p.clone()));
            }
        }

        let expected: BTreeSet<(String, String)> = KNOWN_DRIFT_PARAMS
            .iter()
            .map(|(t, p, _)| (t.to_string(), p.to_string()))
            .collect();

        let unexpected: Vec<&(String, String)> = actual.difference(&expected).collect();
        let stale: Vec<&(String, String)> = expected.difference(&actual).collect();

        assert!(
            unexpected.is_empty(),
            "NEW required-parameter drift between the chat and MCP surfaces: {unexpected:?}\n\
             The same tool now demands different fields depending on which agent surface calls it."
        );
        assert!(
            stale.is_empty(),
            "documented parameter drift no longer exists: {stale:?}\n\
             Delete the entries from KNOWN_DRIFT_PARAMS."
        );
    }

    /// GIVEN the known-drift lists,
    /// WHEN each entry is read,
    /// THEN it carries a non-empty rationale.
    ///
    /// A bare allow-list decays into a list of things nobody remembers agreeing
    /// to. The reason is the part that has to survive.
    #[test]
    fn every_documented_exception_states_a_reason() {
        for (name, why) in INTENTIONAL_CHAT_ONLY
            .iter()
            .chain(INTENTIONAL_MCP_ONLY)
            .chain(KNOWN_DRIFT_NAMES)
        {
            assert!(!why.trim().is_empty(), "{name} has no documented reason");
        }
        for (tool, param, why) in KNOWN_DRIFT_PARAMS {
            assert!(
                !why.trim().is_empty(),
                "{tool}.{param} has no documented reason"
            );
        }
    }
}
