//! AI chat endpoint — LLM proposes, deterministic ledger enforces.
//!
//! `POST /chat` runs a local-LLM tool loop: the model picks ledger capabilities
//! and fills their fields; `dispatch_tool` executes them deterministically and
//! returns structured results. The model never computes balances or touches SQL.
//!
//! The Ollama tool loop is NOT covered by automated tests (non-deterministic,
//! requires the daemon). The deterministic `dispatch_tool` function IS unit-tested
//! in `tests/chat_dispatch.rs`.

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use chrono::NaiveDate;
use ol_domain::Line;
use ol_ledger::{PostError, PostRequest, account_balance, post_journal_entry};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

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
}

/// A single tool invocation with its result.
#[derive(Debug, Serialize, ToSchema)]
pub struct ChatAction {
    pub tool: String,
    pub args: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

/// Response body for `POST /chat`.
#[derive(Debug, Serialize, ToSchema)]
pub struct ChatResponse {
    pub reply: String,
    pub actions: Vec<ChatAction>,
}

// ─── Tool executor ────────────────────────────────────────────────────────────

/// Execute a named ledger tool deterministically.
///
/// The LLM proposes a tool call; this function enforces every invariant by
/// delegating to `ol_ledger` and `ol_process`. The model never computes
/// balances or touches SQL.
pub async fn dispatch_tool(
    pool: &PgPool,
    name: &str,
    args: &serde_json::Value,
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
                idempotency_key: Uuid::new_v4(),
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
        }
    ])
}

// ─── System prompt ────────────────────────────────────────────────────────────

const SYSTEM_PROMPT: &str = "You are the OpenERP assistant. You operate a double-entry \
accounting ledger by calling tools. NEVER compute balances yourself — call \
get_account_balance. To record money movement, call post_journal_entry with lines \
whose debits equal credits within each currency (amounts are integer cents: 100 = 1.00). \
The ledger enforces correctness and will reject anything invalid — if a tool returns an \
error, read it and explain it plainly. Be concise. When you have done what was asked, \
give a one or two sentence summary.";

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

/// Chat with the OpenERP AI assistant.
///
/// The assistant uses tool calls to query and update the ledger.
/// The model never computes balances or touches SQL directly — all ledger
/// operations go through the deterministic `dispatch_tool` executor.
#[utoipa::path(
    post,
    path = "/chat",
    request_body = ChatRequest,
    responses(
        (status = 200, description = "Assistant reply with any tool actions taken", body = ChatResponse),
        (status = 503, description = "LLM backend unavailable", body = ChatResponse),
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

    let client = reqwest::Client::new();
    let mut actions: Vec<ChatAction> = Vec::new();
    let mut reply = String::new();

    // Tool loop — max 6 iterations.
    'outer: for _ in 0..6 {
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
                return (StatusCode::OK, Json(ChatResponse { reply, actions })).into_response();
            }
        };

        let ollama_resp: OllamaResponse = match resp.json().await {
            Ok(r) => r,
            Err(e) => {
                reply = format!(
                    "The AI assistant returned an unexpected response: {e}. \
                     Check that the model is loaded in Ollama."
                );
                return (StatusCode::OK, Json(ChatResponse { reply, actions })).into_response();
            }
        };

        let msg = ollama_resp.message;

        if msg.tool_calls.is_empty() {
            // Final text reply — done.
            reply = msg.content;
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

            let tool_result = dispatch_tool(&pool, &tc.function.name, &args).await;

            let (result_val, error_str, tool_content) = match &tool_result {
                Ok(v) => (Some(v.clone()), None, v.to_string()),
                Err(e) => (None, Some(e.clone()), format!("ERROR: {e}")),
            };

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
        reply =
            "I completed the requested operations. Check the actions list for details.".to_string();
    }

    (StatusCode::OK, Json(ChatResponse { reply, actions })).into_response()
}

// ─── Process dir helper (shared with lib.rs) ──────────────────────────────────

fn processes_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("PROCESSES_DIR") {
        return std::path::PathBuf::from(dir);
    }
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("processes")
}
