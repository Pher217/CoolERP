//! Process-instance endpoints.
//!
//! POST /instances              — start a new process instance (idempotent).
//! POST /instances/{id}/advance — advance an instance by one transition.
//! GET  /instances              — list instances, optionally filtered.
//! GET  /instances/{id}         — instance detail with step log and available transitions.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::{Local, NaiveDate};
use ol_engine::{AdvanceInput, EngineError};
use ol_sdk::{ErrorCode, ErrorEnvelope};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::{err_response, post_error_response};

// ─── Process instances ────────────────────────────────────────────────────────

/// A snapshot of a process instance.
#[derive(Debug, Serialize, ToSchema)]
pub struct InstanceResponse {
    pub id: i64,
    pub process: String,
    pub current_state: String,
    pub status: String,
    pub reference: Option<String>,
    pub context: serde_json::Value,
}

impl From<ol_engine::Instance> for InstanceResponse {
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

/// One row of the step audit log.
#[derive(Debug, Serialize, ToSchema)]
pub struct StepLogResponse {
    pub id: i64,
    pub instance_id: i64,
    pub from_state: String,
    pub to_state: String,
    pub capability: String,
    pub actor: String,
    pub entry_id: Option<i64>,
    pub payload: Option<serde_json::Value>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl From<ol_engine::StepLog> for StepLogResponse {
    fn from(s: ol_engine::StepLog) -> Self {
        Self {
            id: s.id,
            instance_id: s.instance_id,
            from_state: s.from_state,
            to_state: s.to_state,
            capability: s.capability,
            actor: s.actor,
            entry_id: s.entry_id,
            payload: s.payload,
            created_at: s.created_at,
        }
    }
}

/// Posting amounts required by an available transition.
///
/// `required_amount_keys` lists exactly the keys the caller must supply in the
/// `amounts` map. The `credit_roles` field describes every account role this
/// transition credits and is always populated.
#[derive(Debug, Serialize, ToSchema)]
pub struct PostingRequirementResponse {
    /// The debit role name (for reference; the caller always uses the key `"amount"`).
    pub debit_role: String,
    /// Every account role this transition credits, always populated.
    pub credit_roles: Vec<String>,
    /// Exactly the keys the caller must supply in `amounts`.
    pub required_amount_keys: Vec<String>,
}

/// One legal next move from the current state of a process instance.
#[derive(Debug, Serialize, ToSchema)]
pub struct AvailableTransitionResponse {
    /// Pass this capability name to `POST /instances/{id}/advance`.
    pub capability: String,
    /// The state the instance will move to when this transition fires.
    pub to_state: String,
    /// `Some` when this transition requires posting amounts; `null` otherwise.
    pub posting: Option<PostingRequirementResponse>,
}

/// Response for `GET /instances/{id}`.
#[derive(Debug, Serialize, ToSchema)]
pub struct GetInstanceResponse {
    pub instance: InstanceResponse,
    pub steps: Vec<StepLogResponse>,
    /// Legal next capabilities from the current state.  Empty when the instance
    /// is terminal (no outgoing transitions).
    pub available: Vec<AvailableTransitionResponse>,
}

/// Request body for `POST /instances`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct StartInstanceRequest {
    /// Process name, e.g. "order_to_cash".
    pub process: String,
    /// UUID idempotency key — reuse the same key to replay without creating a
    /// duplicate instance. If omitted, a deterministic key is derived from the
    /// payload so identical calls are still idempotent.
    pub idempotency_key: Option<Uuid>,
    /// Optional external reference (order number, customer id, etc.).
    pub reference: Option<String>,
    /// Initial context as a JSON object. Defaults to `{}`.
    #[serde(default)]
    pub context: serde_json::Value,
}

/// Request body for `POST /instances/{id}/advance`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AdvanceInstanceRequest {
    /// Capability (transition label) to execute, e.g. "confirm_order".
    pub capability: String,
    /// Named amounts in integer cents. Required for transitions with a posting_rule.
    #[serde(default)]
    pub amounts: std::collections::HashMap<String, i64>,
    /// JSON object merged into the instance context (shallow merge via `||`).
    #[serde(default)]
    pub context_patch: serde_json::Value,
    /// Accounting date (YYYY-MM-DD). Defaults to today when absent.
    pub entry_date: Option<NaiveDate>,
}

/// Query parameters for `GET /instances`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ListInstancesQuery {
    pub process: Option<String>,
    pub status: Option<String>,
}

/// Map an [`EngineError`] to an HTTP status + [`ErrorEnvelope`].
///
/// The wording is owned by the engine's `#[error(...)]` `Display` impl; this
/// function only chooses the HTTP status and [`ErrorCode`] per variant.
fn engine_error_response(e: EngineError) -> (StatusCode, Json<ErrorEnvelope>) {
    let (status, code) = match e {
        EngineError::ProcessNotFound(_) => (StatusCode::NOT_FOUND, ErrorCode::Validation),
        EngineError::IllegalTransition { .. } => (StatusCode::CONFLICT, ErrorCode::Validation),
        EngineError::InstanceNotActive(_) => (StatusCode::CONFLICT, ErrorCode::Validation),
        EngineError::ConcurrentAdvance => (StatusCode::CONFLICT, ErrorCode::SerializationFailure),
        EngineError::IdempotencyKeyReused(_) => {
            (StatusCode::CONFLICT, ErrorCode::DuplicateIdempotencyKey)
        }
        EngineError::UnknownRole(_) => (StatusCode::UNPROCESSABLE_ENTITY, ErrorCode::Validation),
        EngineError::PostingAmountsRequired { .. } => {
            (StatusCode::UNPROCESSABLE_ENTITY, ErrorCode::Validation)
        }
        EngineError::Unbalanced { .. } => {
            (StatusCode::UNPROCESSABLE_ENTITY, ErrorCode::UnbalancedEntry)
        }
        EngineError::Ledger(inner) => return post_error_response(inner),
        EngineError::Db(_) | EngineError::Io(_) | EngineError::ProcessLoad(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::Internal)
        }
    };
    err_response(status, code, e.to_string())
}

/// Start a new process instance.
#[utoipa::path(
    post,
    path = "/instances",
    request_body = StartInstanceRequest,
    responses(
        (status = 201, description = "Instance created", body = InstanceResponse),
        (status = 200, description = "Instance replayed from idempotency key", body = InstanceResponse),
        (status = 404, description = "Process not found", body = crate::ErrorBody),
        (status = 409, description = "Idempotency key reused with a different payload", body = crate::ErrorBody),
        (status = 500, description = "Database or IO error", body = crate::ErrorBody),
    )
)]
pub async fn start_instance(
    State(pool): State<PgPool>,
    Json(body): Json<StartInstanceRequest>,
) -> impl IntoResponse {
    let idempotency_key = body.idempotency_key.unwrap_or_else(|| {
        ol_engine::derive_start_instance_key(
            &body.process,
            body.reference.as_deref(),
            &body.context,
        )
    });

    match ol_engine::start_instance_with_key(
        &pool,
        &body.process,
        body.reference,
        body.context,
        idempotency_key,
    )
    .await
    {
        Ok(outcome) => {
            let status = if outcome.replayed {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            };
            (status, Json(InstanceResponse::from(outcome.instance))).into_response()
        }
        Err(e) => engine_error_response(e).into_response(),
    }
}

/// Advance a process instance by one transition.
#[utoipa::path(
    post,
    path = "/instances/{id}/advance",
    params(
        ("id" = i64, Path, description = "Instance id")
    ),
    request_body = AdvanceInstanceRequest,
    responses(
        (status = 200, description = "Instance advanced", body = InstanceResponse),
        (status = 404, description = "Instance not found", body = crate::ErrorBody),
        (status = 409, description = "Illegal transition, inactive instance, or concurrent advance", body = crate::ErrorBody),
        (status = 422, description = "Unknown role or unbalanced amounts", body = crate::ErrorBody),
        (status = 500, description = "Database or IO error", body = crate::ErrorBody),
    )
)]
pub async fn advance_instance(
    State(pool): State<PgPool>,
    Path(id): Path<i64>,
    Json(body): Json<AdvanceInstanceRequest>,
) -> impl IntoResponse {
    let entry_date = body.entry_date.unwrap_or_else(|| Local::now().date_naive());

    let input = AdvanceInput {
        amounts: body.amounts,
        context_patch: body.context_patch,
        entry_date,
    };

    match ol_engine::advance_instance(&pool, id, &body.capability, "api", input).await {
        Ok(inst) => (StatusCode::OK, Json(InstanceResponse::from(inst))).into_response(),
        Err(e) => engine_error_response(e).into_response(),
    }
}

/// List process instances, optionally filtered by process name and/or status.
#[utoipa::path(
    get,
    path = "/instances",
    params(
        ("process" = Option<String>, Query, description = "Filter by process name"),
        ("status" = Option<String>, Query, description = "Filter by status: active, completed, cancelled"),
    ),
    responses(
        (status = 200, description = "List of instances", body = Vec<InstanceResponse>),
        (status = 500, description = "Database error", body = crate::ErrorBody),
    )
)]
pub async fn list_instances(
    State(pool): State<PgPool>,
    Query(q): Query<ListInstancesQuery>,
) -> impl IntoResponse {
    match ol_engine::list_instances(&pool, q.process.as_deref(), q.status.as_deref()).await {
        Ok(list) => {
            let resp: Vec<InstanceResponse> = list.into_iter().map(Into::into).collect();
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => engine_error_response(e).into_response(),
    }
}

/// Get a process instance and its full step log.
#[utoipa::path(
    get,
    path = "/instances/{id}",
    params(
        ("id" = i64, Path, description = "Instance id")
    ),
    responses(
        (status = 200, description = "Instance + step log", body = GetInstanceResponse),
        (status = 404, description = "Instance not found", body = crate::ErrorBody),
        (status = 500, description = "Database error", body = crate::ErrorBody),
    )
)]
pub async fn get_instance(State(pool): State<PgPool>, Path(id): Path<i64>) -> impl IntoResponse {
    match ol_engine::get_instance(&pool, id).await {
        Ok(result) => {
            let resp = GetInstanceResponse {
                instance: result.instance.into(),
                steps: result.steps.into_iter().map(Into::into).collect(),
                available: result
                    .available
                    .into_iter()
                    .map(|at| AvailableTransitionResponse {
                        capability: at.capability,
                        to_state: at.to_state,
                        posting: at.posting.map(|p| PostingRequirementResponse {
                            debit_role: p.debit_role,
                            credit_roles: p.credit_roles,
                            required_amount_keys: p.required_amount_keys,
                        }),
                    })
                    .collect(),
            };
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(EngineError::Db(sqlx::Error::RowNotFound)) => err_response(
            StatusCode::NOT_FOUND,
            ErrorCode::Validation,
            format!("instance not found: {id}"),
        )
        .into_response(),
        Err(e) => engine_error_response(e).into_response(),
    }
}
