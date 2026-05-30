//! ol-api — Axum REST API over the ledger, with generated OpenAPI 3.1.
//!
//! Build the router with [`app`]; `main.rs` supplies the database pool and
//! binds the TCP listener.  The app function is kept separate so integration
//! tests can construct the router without a real listener.
//!
//! Auth is intentionally absent in this skeleton (ADR-008 dev token).
//! TODO: OAuth PKCE (ADR-006)

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use chrono::NaiveDate;
use ol_domain::Line;
use ol_ledger::{PostError, PostRequest, PostResult, account_balance, post_journal_entry};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
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
        .routes(routes!(create_journal_entry))
        .split_for_parts();

    router
        .route(
            "/api-docs/openapi.json",
            get(move || async move { Json(api) }),
        )
        .with_state(pool)
}

// ─── OpenAPI root doc ─────────────────────────────────────────────────────────

#[derive(OpenApi)]
#[openapi(
    info(
        title = "OpenLedger API",
        version = "0.1.0",
        description = "Double-entry ledger REST API. Money amounts are integer cents (i64). No floats."
    ),
    components(schemas(
        HealthResponse,
        BalanceResponse,
        CreateJournalEntryRequest,
        LineRequest,
        PostResultResponse,
        ApiError,
    ))
)]
struct ApiDoc;

// ─── Shared types ─────────────────────────────────────────────────────────────

/// JSON error body returned on all non-2xx responses.
#[derive(Debug, Serialize, ToSchema)]
pub struct ApiError {
    pub code: String,
    pub message: String,
}

impl ApiError {
    fn with_status(
        status: StatusCode,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> (StatusCode, Json<Self>) {
        (
            status,
            Json(Self {
                code: code.into(),
                message: message.into(),
            }),
        )
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
        (status = 404, description = "Account not found", body = ApiError),
        (status = 500, description = "Database error", body = ApiError),
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
                debits: bal.debits,
                credits: bal.credits,
                balance: bal.balance,
            }),
        )
            .into_response(),
        Err(PostError::AccountNotFound(c)) => ApiError::with_status(
            StatusCode::NOT_FOUND,
            "ACCOUNT_NOT_FOUND",
            format!("account not found: {c}"),
        )
        .into_response(),
        Err(e) => {
            ApiError::with_status(StatusCode::INTERNAL_SERVER_ERROR, "DB_ERROR", e.to_string())
                .into_response()
        }
    }
}

// ─── POST /journal-entries ────────────────────────────────────────────────────

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
}

/// Request body for `POST /journal-entries`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateJournalEntryRequest {
    pub idempotency_key: Uuid,
    pub journal_code: String,
    /// ISO-8601 date (YYYY-MM-DD).
    pub entry_date: NaiveDate,
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
        (status = 404, description = "Account or journal not found", body = ApiError),
        (status = 409, description = "Serialization failure after retries exhausted", body = ApiError),
        (status = 422, description = "Domain error (unbalanced entry, invalid lines)", body = ApiError),
        (status = 500, description = "Database error", body = ApiError),
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
        memo: body.memo,
        reference: body.reference,
        actor: body.actor,
        lines: body
            .lines
            .into_iter()
            .map(|l| Line {
                account_code: l.account_code,
                debit: l.debit,
                credit: l.credit,
            })
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
        Err(PostError::Domain(e)) => ApiError::with_status(
            StatusCode::UNPROCESSABLE_ENTITY,
            "DOMAIN_ERROR",
            e.to_string(),
        )
        .into_response(),
        Err(PostError::Unbalanced { message }) => ApiError::with_status(
            StatusCode::UNPROCESSABLE_ENTITY,
            "UNBALANCED_ENTRY",
            message,
        )
        .into_response(),
        Err(PostError::AccountNotFound(c)) => ApiError::with_status(
            StatusCode::NOT_FOUND,
            "ACCOUNT_NOT_FOUND",
            format!("account not found: {c}"),
        )
        .into_response(),
        Err(PostError::JournalNotFound(j)) => ApiError::with_status(
            StatusCode::NOT_FOUND,
            "JOURNAL_NOT_FOUND",
            format!("journal not found: {j}"),
        )
        .into_response(),
        Err(PostError::Serialization(attempts)) => ApiError::with_status(
            StatusCode::CONFLICT,
            "SERIALIZATION_FAILURE",
            format!("exhausted {attempts} retry attempts"),
        )
        .into_response(),
        Err(PostError::AppendOnly { message }) => {
            ApiError::with_status(StatusCode::UNPROCESSABLE_ENTITY, "APPEND_ONLY", message)
                .into_response()
        }
        Err(PostError::Db(e)) => {
            ApiError::with_status(StatusCode::INTERNAL_SERVER_ERROR, "DB_ERROR", e.to_string())
                .into_response()
        }
    }
}
