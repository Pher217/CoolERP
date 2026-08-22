//! ol-api — Axum REST API over the ledger, with generated OpenAPI 3.1.
//!
//! Build the router with [`app`]; `main.rs` supplies the database pool and
//! binds the TCP listener.  The app function is kept separate so integration
//! tests can construct the router without a real listener.
//!
//! Auth is intentionally absent in this skeleton (ADR-022 honest security posture).
//! TODO: OAuth PKCE (ADR-006)

pub mod chat;
pub mod instances;
pub mod inventory;
pub mod metrics;
pub mod processes;
pub mod reports;

// The inventory handlers and their request/response types live in
// `inventory`, but the integration tests import `ReceiveStockRequest` and
// `receive_stock_core` from the crate root — re-export them so the move does
// not change the public surface the tests rely on.
pub use inventory::{ReceiveStockRequest, receive_stock_core};

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderValue, Method, StatusCode, header},
    response::IntoResponse,
    routing::get,
};
use chrono::NaiveDate;
use ol_domain::Line;
use ol_ledger::{
    PostError, PostRequest, PostResult, account_balance, account_balances_all, post_journal_entry,
};
use ol_sdk::{ApiError, ErrorCode, ErrorEnvelope};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tower_http::cors::{AllowOrigin, CorsLayer};
use utoipa::{OpenApi, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;
use uuid::Uuid;

// ─── App builder ─────────────────────────────────────────────────────────────

/// Construct the Axum router.  Pass the returned router to
/// `axum::serve` in `main`.  The pool is stored as shared state.
pub fn app(pool: PgPool) -> axum::Router {
    let (router, api) = OpenApiRouter::with_openapi(ApiDoc::openapi())
        .routes(routes!(health))
        .routes(routes!(get_balance))
        .routes(routes!(get_balances_all))
        .routes(routes!(create_journal_entry))
        .routes(routes!(processes::list_processes))
        .routes(routes!(processes::get_process))
        .routes(routes!(inventory::get_inventory))
        .routes(routes!(inventory::receive_stock))
        .routes(routes!(instances::start_instance))
        .routes(routes!(instances::advance_instance))
        .routes(routes!(instances::list_instances))
        .routes(routes!(instances::get_instance))
        .routes(routes!(chat::chat))
        .routes(routes!(metrics::get_overview))
        .routes(routes!(metrics::get_ar_aging))
        .routes(routes!(reports::trial_balance))
        .routes(routes!(reports::subledger_reconciliation))
        .split_for_parts();

    router
        .route(
            "/api-docs/openapi.json",
            get(move || async move { Json(api) }),
        )
        // Explicit CORS allowlist for the browser SPA. Defaults to the Vite dev
        // origin; override with CORS_ALLOWED_ORIGINS (comma-separated).
        .layer(cors_layer())
        .with_state(pool)
}

/// Default origin the Vite dev UI is served from.
const DEFAULT_CORS_ORIGIN: &str = "http://localhost:5173";

/// Build a CORS layer scoped to the configured origin allowlist.
///
/// Defaults to [`DEFAULT_CORS_ORIGIN`]. Override with `CORS_ALLOWED_ORIGINS` as a
/// comma-separated list of origins. Invalid values panic on startup.
fn cors_layer() -> CorsLayer {
    let origins: Vec<HeaderValue> = std::env::var("CORS_ALLOWED_ORIGINS")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(HeaderValue::from_str)
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_else(|| Ok(vec![HeaderValue::from_static(DEFAULT_CORS_ORIGIN)]))
        .expect("CORS_ALLOWED_ORIGINS contains invalid origin header values");

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::CONTENT_TYPE, header::ACCEPT])
}

// ─── OpenAPI root doc ─────────────────────────────────────────────────────────

#[derive(OpenApi)]
#[openapi(
    info(
        title = "CoolERP API",
        version = "0.1.0",
        description = "Double-entry ledger REST API. Money amounts are integer cents (i64). No floats."
    ),
    components(schemas(
        HealthResponse,
        BalanceResponse,
        CreateJournalEntryRequest,
        LineRequest,
        PostResultResponse,
        ErrorBody,
        processes::ListProcessesResponse,
        processes::ProcessResponse,
        processes::TransitionResponse,
        processes::PostingRuleResponse,
        processes::StepDetailResponse,
        processes::StepFieldResponse,
        inventory::InventoryItem,
        inventory::InventoryResponse,
        inventory::ReceiveStockRequest,
        inventory::ReceiveStockResponse,
        chat::ChatRequest,
        chat::ChatTurn,
        chat::ChatResponse,
        chat::ChatAction,
        chat::ViewDirective,
        metrics::OverviewResponse,
        metrics::AgingBucket,
        metrics::ArAgingResponse,
        reports::TrialBalanceLine,
        reports::TrialBalanceResponse,
        reports::SubledgerSection,
        reports::SubledgerReconciliationResponse,
        instances::InstanceResponse,
        instances::StartInstanceRequest,
        instances::AdvanceInstanceRequest,
        instances::StepLogResponse,
        instances::GetInstanceResponse,
        instances::ListInstancesQuery,
        instances::PostingRequirementResponse,
        instances::AvailableTransitionResponse,
    ))
)]
struct ApiDoc;

// ─── Error helpers ───────────────────────────────────────────────────────────

/// OpenAPI schema for the error envelope body.
///
/// `ol_sdk::ErrorEnvelope` is not `ToSchema`-derived (it lives in the SDK crate
/// without a utoipa dependency), so we register this local wrapper in the OpenAPI
/// component map.  The actual wire encoding is [`ErrorEnvelope`] — identical shape.
#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBody {
    pub error: ErrorBodyInner,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBodyInner {
    pub code: String,
    pub message: String,
}

/// Build an [`ErrorEnvelope`] and pair it with an HTTP status, ready for
/// [`IntoResponse`].
fn err_response(
    status: StatusCode,
    code: ErrorCode,
    message: impl Into<String>,
) -> (StatusCode, Json<ErrorEnvelope>) {
    (
        status,
        Json(ErrorEnvelope::new(ApiError::new(code, message))),
    )
}

/// Map a [`PostError`] to an HTTP status + canonical [`ErrorEnvelope`].
///
/// Mapping table:
///
/// | PostError variant      | HTTP | ErrorCode            |
/// |------------------------|------|----------------------|
/// | Domain(_)              | 422  | UnbalancedEntry      |
/// | Unbalanced{..}         | 422  | UnbalancedEntry      |
/// | AppendOnly{..}         | 422  | AppendOnly           |
/// | AccountNotFound        | 404  | AccountNotFound      |
/// | JournalNotFound        | 404  | JournalNotFound      |
/// | Serialization(_)       | 409  | SerializationFailure |
/// | Db(_)                  | 500  | Internal             |
///
/// `Domain` maps to `UnbalancedEntry` because `ol_domain::LedgerError` covers
/// unbalanced totals, fewer than two lines, and double-sided lines — all
/// structural balance violations.  A dedicated `Validation` code would hide
/// the more precise information already in the message.
fn post_error_response(e: PostError) -> (StatusCode, Json<ErrorEnvelope>) {
    match e {
        PostError::Domain(inner) => err_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::UnbalancedEntry,
            inner.to_string(),
        ),
        PostError::Unbalanced { message } => err_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::UnbalancedEntry,
            message,
        ),
        PostError::AppendOnly { message } => err_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::AppendOnly,
            message,
        ),
        PostError::AccountNotFound(code) => err_response(
            StatusCode::NOT_FOUND,
            ErrorCode::AccountNotFound,
            format!("account not found: {code}"),
        ),
        PostError::JournalNotFound(code) => err_response(
            StatusCode::NOT_FOUND,
            ErrorCode::JournalNotFound,
            format!("journal not found: {code}"),
        ),
        PostError::PeriodClosed { message } => {
            err_response(StatusCode::CONFLICT, ErrorCode::PeriodClosed, message)
        }
        PostError::PeriodOverlap { message } => {
            err_response(StatusCode::CONFLICT, ErrorCode::Validation, message)
        }
        PostError::PeriodNotFound(code) => err_response(
            StatusCode::NOT_FOUND,
            ErrorCode::Validation,
            format!("fiscal period not found: {code}"),
        ),
        PostError::Serialization(attempts) => err_response(
            StatusCode::CONFLICT,
            ErrorCode::SerializationFailure,
            format!("exhausted {attempts} retry attempts"),
        ),
        PostError::IdempotencyKeyReused(message) => err_response(
            StatusCode::CONFLICT,
            ErrorCode::DuplicateIdempotencyKey,
            message,
        ),
        PostError::Db(inner) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            inner.to_string(),
        ),
    }
}

// ─── GET /health ──────────────────────────────────────────────────────────────

/// Health-check response.
#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
}

/// Check service health.
#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "Service is up", body = HealthResponse)
    )
)]
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
    })
}

// ─── GET /accounts/{code}/balance ─────────────────────────────────────────────

/// Account balance. All amounts are integer cents (i64).
#[derive(Debug, Serialize, ToSchema)]
pub struct BalanceResponse {
    /// Account code (e.g. "1000").
    pub account_code: String,
    /// ISO-4217 currency code (e.g. "EUR").
    pub currency: String,
    /// Total debits posted to this account, in cents.
    #[schema(value_type = i64, format = Int64)]
    pub debits: i64,
    /// Total credits posted to this account, in cents.
    #[schema(value_type = i64, format = Int64)]
    pub credits: i64,
    /// Type-normalised signed balance in cents (debit-positive for asset/expense,
    /// credit-positive for liability/equity/income).
    #[schema(value_type = i64, format = Int64)]
    pub balance: i64,
}

/// Get balance for an account by code.
#[utoipa::path(
    get,
    path = "/accounts/{code}/balance",
    params(
        ("code" = String, Path, description = "Account code, e.g. \"1000\"")
    ),
    responses(
        (status = 200, description = "Account balance", body = BalanceResponse),
        (status = 404, description = "Account not found", body = ErrorBody),
        (status = 500, description = "Database error", body = ErrorBody),
    )
)]
pub async fn get_balance(
    State(pool): State<PgPool>,
    Path(code): Path<String>,
) -> impl IntoResponse {
    match account_balance(&pool, &code).await {
        Ok(bal) => (
            StatusCode::OK,
            Json(BalanceResponse {
                account_code: bal.account_code,
                currency: bal.currency,
                debits: bal.debits,
                credits: bal.credits,
                balance: bal.balance,
            }),
        )
            .into_response(),
        Err(e) => post_error_response(e).into_response(),
    }
}

/// Get all per-currency balances for an account by code.
#[utoipa::path(
    get,
    path = "/accounts/{code}/balances",
    params(
        ("code" = String, Path, description = "Account code, e.g. \"1000\"")
    ),
    responses(
        (status = 200, description = "All currency balances for the account", body = Vec<BalanceResponse>),
        (status = 404, description = "Account not found", body = ErrorBody),
        (status = 500, description = "Database error", body = ErrorBody),
    )
)]
pub async fn get_balances_all(
    State(pool): State<PgPool>,
    Path(code): Path<String>,
) -> impl IntoResponse {
    match account_balances_all(&pool, &code).await {
        Ok(bals) => {
            let resp: Vec<BalanceResponse> = bals
                .into_iter()
                .map(|b| BalanceResponse {
                    account_code: b.account_code,
                    currency: b.currency,
                    debits: b.debits,
                    credits: b.credits,
                    balance: b.balance,
                })
                .collect();
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => post_error_response(e).into_response(),
    }
}

// ─── POST /journal-entries ────────────────────────────────────────────────────

fn default_currency() -> String {
    "EUR".into()
}

/// A single journal line in a POST request.  Exactly one of `debit`/`credit`
/// must be non-zero; both are integer cents (i64).
#[derive(Debug, Deserialize, ToSchema)]
pub struct LineRequest {
    pub account_code: String,
    /// Debit amount in cents. Mutually exclusive with `credit`.
    #[schema(value_type = i64, format = Int64)]
    pub debit: i64,
    /// Credit amount in cents. Mutually exclusive with `debit`.
    #[schema(value_type = i64, format = Int64)]
    pub credit: i64,
    /// ISO-4217 currency code. Defaults to "EUR" when absent.
    #[serde(default = "default_currency")]
    pub currency: String,
}

/// Request body for `POST /journal-entries`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateJournalEntryRequest {
    pub idempotency_key: Uuid,
    pub journal_code: String,
    /// ISO-8601 date (YYYY-MM-DD).
    pub entry_date: NaiveDate,
    /// Economic date determining the fiscal period. Defaults to entry_date when absent.
    #[serde(default)]
    pub effective_date: Option<NaiveDate>,
    pub memo: Option<String>,
    pub reference: Option<String>,
    /// Audit actor — the subject of the calling token.
    pub actor: String,
    pub lines: Vec<LineRequest>,
}

/// Successful result of a `POST /journal-entries` call.
#[derive(Debug, Serialize, ToSchema)]
pub struct PostResultResponse {
    /// Database id of the created (or replayed) journal entry.
    pub entry_id: i64,
    pub balanced: bool,
    /// `true` when the idempotency key was already used; the original entry is
    /// returned unchanged.
    pub replayed: bool,
}

impl From<PostResult> for PostResultResponse {
    fn from(r: PostResult) -> Self {
        Self {
            entry_id: r.entry_id,
            balanced: r.balanced,
            replayed: r.replayed,
        }
    }
}

/// Post a balanced journal entry (idempotent via `idempotency_key`).
///
/// Returns the created entry, or — if the idempotency key was already used —
/// the original entry with `replayed: true`.
// TODO: OAuth (ADR-006)
#[utoipa::path(
    post,
    path = "/journal-entries",
    request_body = CreateJournalEntryRequest,
    responses(
        (status = 201, description = "Entry created", body = PostResultResponse),
        (status = 200, description = "Idempotent replay of existing entry", body = PostResultResponse),
        (status = 404, description = "Account or journal not found", body = ErrorBody),
        (status = 409, description = "Serialization failure after retries exhausted", body = ErrorBody),
        (status = 422, description = "Domain error (unbalanced entry, invalid lines)", body = ErrorBody),
        (status = 500, description = "Database error", body = ErrorBody),
    )
)]
pub async fn create_journal_entry(
    State(pool): State<PgPool>,
    Json(body): Json<CreateJournalEntryRequest>,
) -> impl IntoResponse {
    let req = PostRequest {
        idempotency_key: body.idempotency_key,
        journal_code: body.journal_code,
        entry_date: body.entry_date,
        effective_date: body.effective_date,
        memo: body.memo,
        reference: body.reference,
        actor: body.actor,
        lines: body
            .lines
            .into_iter()
            .map(|l| Line::in_currency(l.account_code, l.debit, l.credit, l.currency))
            .collect(),
    };

    match post_journal_entry(&pool, &req).await {
        Ok(result) => {
            let status = if result.replayed {
                StatusCode::OK
            } else {
                StatusCode::CREATED
            };
            (status, Json(PostResultResponse::from(result))).into_response()
        }
        Err(e) => post_error_response(e).into_response(),
    }
}
