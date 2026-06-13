//! ol-api — Axum REST API over the ledger, with generated OpenAPI 3.1.
//!
//! Build the router with [`app`]; `main.rs` supplies the database pool and
//! binds the TCP listener.  The app function is kept separate so integration
//! tests can construct the router without a real listener.
//!
//! Auth is intentionally absent in this skeleton (ADR-008 dev token).
//! TODO: OAuth PKCE (ADR-006)

pub mod chat;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
};
use chrono::NaiveDate;
use ol_domain::Line;
use ol_ledger::{
    PostError, PostRequest, PostResult, account_balance, account_balances_all, post_journal_entry,
};
use ol_process::Process;
use ol_sdk::{ApiError, ErrorCode, ErrorEnvelope};
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
        .routes(routes!(get_balances_all))
        .routes(routes!(create_journal_entry))
        .routes(routes!(list_processes))
        .routes(routes!(get_process))
        .routes(routes!(chat::chat))
        .split_for_parts();

    router
        .route(
            "/api-docs/openapi.json",
            get(move || async move { Json(api) }),
        )
        // Permissive CORS for local/dev (browser SPA on a different origin).
        // Tighten to the known UI origin when the OAuth layer lands (ADR-006).
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(pool)
}

// ─── OpenAPI root doc ─────────────────────────────────────────────────────────

#[derive(OpenApi)]
#[openapi(
    info(
        title = "OpenERP API",
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
        ListProcessesResponse,
        ProcessResponse,
        TransitionResponse,
        chat::ChatRequest,
        chat::ChatTurn,
        chat::ChatResponse,
        chat::ChatAction,
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

// ─── Process directory helpers ────────────────────────────────────────────────

/// Resolve the directory where process YAML files are stored.
///
/// Priority order:
/// 1. `PROCESSES_DIR` environment variable (runtime override).
/// 2. `<manifest_dir>/../../processes` — works in both a local dev checkout and
///    inside `.claude/worktrees/…`, since `processes/` sits two levels above
///    `crates/ol-api` in both layouts.
fn processes_dir() -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("PROCESSES_DIR") {
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
        (status = 500, description = "Failed to read process directory", body = ErrorBody),
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
    /// Mermaid `stateDiagram-v2` source for the full workflow.
    pub mermaid: String,
}

/// A single state transition.
#[derive(Debug, Serialize, ToSchema)]
pub struct TransitionResponse {
    pub from: String,
    pub to: String,
    pub capability: Option<String>,
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
        (status = 404, description = "Process not found", body = ErrorBody),
        (status = 500, description = "Failed to load or parse process YAML", body = ErrorBody),
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
            })
            .collect(),
        mermaid,
    })
    .into_response()
}
