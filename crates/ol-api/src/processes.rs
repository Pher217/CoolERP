//! Process-definition endpoints.
//!
//! GET /processes        — list available process names (from YAML filenames).
//! GET /processes/{name} — full process detail with the rendered Mermaid diagram.

use axum::{Json, extract::Path, http::StatusCode, response::IntoResponse};
use ol_process::Process;
use ol_sdk::ErrorCode;
use serde::Serialize;
use utoipa::ToSchema;

use crate::err_response;

// ─── Process directory helpers ────────────────────────────────────────────────

/// Resolve the directory where process YAML files are stored.
///
/// Priority order:
/// 1. `OL_PROCESSES_DIR` environment variable (runtime override).
/// 2. `<manifest_dir>/../../processes` — works in both a local dev checkout and
///    inside `.claude/worktrees/…`, since `processes/` sits two levels above
///    `crates/ol-api` in both layouts.
fn processes_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("OL_PROCESSES_DIR") {
        return std::path::PathBuf::from(dir);
    }
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("processes")
}

// ─── GET /processes ───────────────────────────────────────────────────────────

/// List of process names available on the server.
#[derive(Debug, Serialize, ToSchema)]
pub struct ListProcessesResponse {
    pub processes: Vec<String>,
}

/// List all known process names.
///
/// Names are derived from YAML filenames in `processes/` (stem without `.yaml`),
/// returned in alphabetical order.
#[utoipa::path(
    get,
    path = "/processes",
    responses(
        (status = 200, description = "List of process names", body = ListProcessesResponse),
        (status = 500, description = "Failed to read process directory", body = crate::ErrorBody),
    )
)]
pub async fn list_processes() -> impl IntoResponse {
    let dir = processes_dir();

    let read_dir = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(e) => {
            return err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                format!("failed to read processes directory: {e}"),
            )
            .into_response();
        }
    };

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

    Json(ListProcessesResponse { processes: names }).into_response()
}

// ─── GET /processes/{name} ────────────────────────────────────────────────────

/// Full detail of a single process, including the rendered Mermaid diagram.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProcessResponse {
    /// Process identifier.
    pub process: String,
    /// Ordered list of valid states.
    pub states: Vec<String>,
    /// State transitions with optional capability labels.
    pub transitions: Vec<TransitionResponse>,
    /// Optional rich UI metadata for each process step.
    pub steps: Vec<StepDetailResponse>,
    /// Mermaid `stateDiagram-v2` source for the full workflow.
    pub mermaid: String,
}

/// A single state transition.
#[derive(Debug, Serialize, ToSchema)]
pub struct TransitionResponse {
    pub from: String,
    pub to: String,
    pub capability: Option<String>,
    /// Guard expressions that must pass before this transition fires.
    #[serde(default)]
    pub guards: Vec<String>,
    /// GL posting rule triggered when this transition fires.
    pub posting_rule: Option<PostingRuleResponse>,
}

/// GL posting rule: debit one account, credit one or more accounts.
#[derive(Debug, Serialize, ToSchema)]
pub struct PostingRuleResponse {
    /// Account code to debit.
    pub debit: String,
    /// Account code(s) to credit.  Multiple accounts are joined with ", ".
    pub credit: String,
}

/// Rich UI metadata for a process state.
#[derive(Debug, Serialize, ToSchema)]
pub struct StepDetailResponse {
    pub state: String,
    pub description: Option<String>,
    pub fields: Vec<StepFieldResponse>,
    pub documents: Vec<String>,
    pub gates: Vec<String>,
    pub kpis: Vec<String>,
}

/// Field captured while a process step is active.
#[derive(Debug, Serialize, ToSchema)]
pub struct StepFieldResponse {
    pub name: String,
    pub label: String,
    pub field_type: String,
    pub required: bool,
}

/// Get a process by name.
#[utoipa::path(
    get,
    path = "/processes/{name}",
    params(
        ("name" = String, Path, description = "Process name, e.g. \"customer_invoice\"")
    ),
    responses(
        (status = 200, description = "Process detail", body = ProcessResponse),
        (status = 404, description = "Process not found", body = crate::ErrorBody),
        (status = 500, description = "Failed to load or parse process YAML", body = crate::ErrorBody),
    )
)]
pub async fn get_process(Path(name): Path<String>) -> impl IntoResponse {
    let path = processes_dir().join(format!("{name}.yaml"));

    let yaml = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return err_response(
                StatusCode::NOT_FOUND,
                ErrorCode::Validation,
                format!("process not found: {name}"),
            )
            .into_response();
        }
        Err(e) => {
            return err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                format!("failed to read process file: {e}"),
            )
            .into_response();
        }
    };

    let process = match Process::from_yaml(&yaml) {
        Ok(p) => p,
        Err(e) => {
            return err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                format!("failed to parse process YAML: {e}"),
            )
            .into_response();
        }
    };

    let mermaid = process.to_mermaid(None);

    Json(ProcessResponse {
        process: process.process.clone(),
        states: process.states.clone(),
        transitions: process
            .transitions
            .iter()
            .map(|t| TransitionResponse {
                from: t.from.clone(),
                to: t.to.clone(),
                capability: t.capability.clone(),
                guards: t.guards.clone().unwrap_or_default(),
                posting_rule: t.posting_rule.as_ref().map(|pr| PostingRuleResponse {
                    debit: pr.debit.clone(),
                    credit: match &pr.credit {
                        ol_process::CreditTarget::Single(s) => s.clone(),
                        ol_process::CreditTarget::Multiple(v) => v.join(", "),
                    },
                }),
            })
            .collect(),
        steps: process
            .steps
            .iter()
            .map(|s| StepDetailResponse {
                state: s.state.clone(),
                description: s.description.clone(),
                fields: s
                    .fields
                    .iter()
                    .map(|f| StepFieldResponse {
                        name: f.name.clone(),
                        label: f.label.clone(),
                        field_type: f.field_type.clone(),
                        required: f.required,
                    })
                    .collect(),
                documents: s.documents.clone(),
                gates: s.gates.clone(),
                kpis: s.kpis.clone(),
            })
            .collect(),
        mermaid,
    })
    .into_response()
}
