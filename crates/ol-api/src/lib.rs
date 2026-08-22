//! ol-api — Axum REST API over the ledger, with generated OpenAPI 3.1.
//!
//! Build the router with [`app`]; `main.rs` supplies the database pool and
//! binds the TCP listener.  The app function is kept separate so integration
//! tests can construct the router without a real listener.
//!
//! Auth is intentionally absent in this skeleton (ADR-022 honest security posture).
//! TODO: OAuth PKCE (ADR-006)

pub mod chat;
pub mod metrics;
pub mod reports;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderValue, Method, StatusCode, header},
    response::IntoResponse,
    routing::get,
};
use chrono::{Local, NaiveDate};
use ol_domain::Line;
use ol_engine::{AdvanceInput, EngineError};
use ol_ledger::{
    PostError, PostRequest, PostResult, account_balance, account_balances_all, post_journal_entry,
};
use ol_process::Process;
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
        .routes(routes!(list_processes))
        .routes(routes!(get_process))
        .routes(routes!(get_inventory))
        .routes(routes!(receive_stock))
        .routes(routes!(start_instance))
        .routes(routes!(advance_instance))
        .routes(routes!(list_instances))
        .routes(routes!(get_instance))
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
        ListProcessesResponse,
        ProcessResponse,
        TransitionResponse,
        PostingRuleResponse,
        StepDetailResponse,
        StepFieldResponse,
        InventoryItem,
        InventoryResponse,
        ReceiveStockRequest,
        ReceiveStockResponse,
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
        InstanceResponse,
        StartInstanceRequest,
        AdvanceInstanceRequest,
        StepLogResponse,
        GetInstanceResponse,
        ListInstancesQuery,
        PostingRequirementResponse,
        AvailableTransitionResponse,
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

// ─── Inventory ───────────────────────────────────────────────────────────────

/// One inventory row with a computed on-hand quantity.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct InventoryItem {
    pub sku: String,
    pub name: String,
    /// Net on-hand quantity as a string.  Computed from stock_moves:
    /// receipts (to_location IS NOT NULL) minus issues (from_location IS NOT NULL).
    pub on_hand: String,
}

/// Response body for `GET /inventory`.
#[derive(Debug, Serialize, ToSchema)]
pub struct InventoryResponse {
    pub items: Vec<InventoryItem>,
}

/// Reusable inventory-on-hand query.  Items with no stock_moves report "0".
pub async fn list_inventory_rows(pool: &PgPool) -> Result<Vec<InventoryItem>, sqlx::Error> {
    let rows = sqlx::query_as!(
        InventoryItem,
        r#"
        SELECT
            i.sku,
            i.name,
            (
                COALESCE(
                    SUM(CASE WHEN sm.to_location IS NOT NULL THEN sm.qty ELSE 0 END), 0::numeric
                ) - COALESCE(
                    SUM(CASE WHEN sm.from_location IS NOT NULL THEN sm.qty ELSE 0 END), 0::numeric
                )
            )::numeric(20, 4)::text AS "on_hand!: String"
        FROM items i
        LEFT JOIN stock_moves sm ON sm.item_id = i.id
        GROUP BY i.id, i.sku, i.name
        ORDER BY i.sku
        "#
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

/// List all inventory items with their on-hand quantities.
#[utoipa::path(
    get,
    path = "/inventory",
    responses(
        (status = 200, description = "Inventory list", body = InventoryResponse),
        (status = 500, description = "Database error", body = ErrorBody),
    )
)]
pub async fn get_inventory(State(pool): State<PgPool>) -> impl IntoResponse {
    match list_inventory_rows(&pool).await {
        Ok(items) => (StatusCode::OK, Json(InventoryResponse { items })).into_response(),
        Err(e) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            format!("database error: {e}"),
        )
        .into_response(),
    }
}

/// Request body for `POST /inventory/receive`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct ReceiveStockRequest {
    /// Caller-supplied key making the receipt retry-safe. Re-submitting the same
    /// key returns the original `move_id` with `replayed: true` (#90).
    pub idempotency_key: Uuid,
    pub sku: String,
    pub location_code: String,
    /// Quantity as a decimal string, parsed by PostgreSQL as NUMERIC.
    pub qty: String,
    /// Optional unit cost as a decimal string, parsed by PostgreSQL as NUMERIC.
    pub unit_cost: Option<String>,
    /// Audit actor — recorded in `events`, exactly as a ledger post is.
    pub actor: String,
}

/// Response body for `POST /inventory/receive`.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ReceiveStockResponse {
    pub move_id: i64,
    pub sku: String,
    pub qty: String,
    /// `true` when the idempotency key was already used; the original move is
    /// returned unchanged and no second stock move was created.
    #[serde(default)]
    pub replayed: bool,
}

/// Operation name for inventory receipts in `idempotency_keys.operation` and
/// `events.capability`. Mirrors `ol_ledger`'s `post_journal_entry`.
const RECEIVE_STOCK_OPERATION: &str = "receive_stock";

/// Shared receipt logic used by the REST handler and the chat tool executor.
pub async fn receive_stock_core(
    pool: &PgPool,
    body: ReceiveStockRequest,
) -> Result<ReceiveStockResponse, String> {
    let request_hash = receive_request_hash(&body);

    let item_id: i64 = sqlx::query_scalar!("SELECT id FROM items WHERE sku = $1", body.sku)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("database error: {e}"))?
        .ok_or_else(|| format!("item not found: {}", body.sku))?;

    let location_id: i64 = sqlx::query_scalar!(
        "SELECT id FROM locations WHERE code = $1",
        body.location_code
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| format!("database error: {e}"))?
    .ok_or_else(|| format!("location not found: {}", body.location_code))?;

    // Claim the idempotency key, insert the move, and record the audit event in
    // ONE transaction — the same shape `ol_ledger::post_journal_entry` uses.
    // Nothing new is invented here: `idempotency_keys` and `events` are already
    // generic over `operation`/`capability`, so no migration is needed. Inventory
    // simply never adopted the mechanism (#90).
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| format!("database error: {e}"))?;

    // RETURNING 1 is Some(_) iff we inserted, i.e. we own this receipt.
    let claimed: Option<i32> = sqlx::query_scalar(
        "INSERT INTO idempotency_keys (operation, idempotency_key, request_hash) \
         VALUES ($1, $2, $3) ON CONFLICT DO NOTHING RETURNING 1",
    )
    .bind(RECEIVE_STOCK_OPERATION)
    .bind(body.idempotency_key)
    .bind(&request_hash)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| format!("database error: {e}"))?;

    if claimed.is_none() {
        // Replay. Read the stored result on the pool rather than inside this
        // transaction: the owning transaction may have committed after our
        // snapshot was taken, and a NULL result means the owner is still in
        // flight rather than that the receipt failed.
        let _ = tx.rollback().await;
        let stored: Option<(Option<String>, Option<serde_json::Value>)> = sqlx::query_as(
            "SELECT request_hash, result FROM idempotency_keys WHERE operation = $1 AND idempotency_key = $2",
        )
        .bind(RECEIVE_STOCK_OPERATION)
        .bind(body.idempotency_key)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("database error: {e}"))?;

        return match stored {
            Some((Some(stored_hash), Some(json))) if stored_hash == request_hash => {
                let mut result: ReceiveStockResponse = serde_json::from_value(json)
                    .map_err(|e| format!("could not decode stored receipt: {e}"))?;
                result.replayed = true;
                Ok(result)
            }
            Some((Some(stored_hash), None)) if stored_hash == request_hash => {
                Err("a receipt with this idempotency key is still in flight".to_string())
            }
            Some((Some(_), _)) | Some((None, _)) => Err(
                "IDEMPOTENCY_KEY_REUSED: idempotency key already used with a different payload"
                    .to_string(),
            ),
            None => Err("a receipt with this idempotency key is still in flight".to_string()),
        };
    }

    let move_id: i64 = sqlx::query_scalar::<_, i64>(
        r#"
        INSERT INTO stock_moves (item_id, qty, to_location, unit_cost)
        VALUES ($1, $2::numeric, $3, $4::numeric)
        RETURNING id
        "#,
    )
    .bind(item_id)
    .bind(body.qty.clone())
    .bind(location_id)
    .bind(body.unit_cost.clone().filter(|s| !s.is_empty()))
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| format!("database error: {e}"))?;

    // Attribution goes in `events`, not a new `stock_moves.actor` column: that is
    // where the ledger records it (`journal_entries` has no actor column either),
    // and a second convention would be worse than none. Before this, a stock
    // movement produced NO audit row at all.
    sqlx::query(
        "INSERT INTO events (actor, capability, inputs_hash, entity_id, result_ids) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(&body.actor)
    .bind(RECEIVE_STOCK_OPERATION)
    .bind(receipt_inputs_hash(&body))
    .bind(move_id.to_string())
    .bind(vec![move_id])
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("database error: {e}"))?;

    let response = ReceiveStockResponse {
        move_id,
        sku: body.sku.clone(),
        qty: body.qty.clone(),
        replayed: false,
    };

    // Store the result in the SAME transaction as the claim, so a visible claim
    // row always has a non-NULL result.
    sqlx::query(
        "UPDATE idempotency_keys SET result = $3 WHERE operation = $1 AND idempotency_key = $2",
    )
    .bind(RECEIVE_STOCK_OPERATION)
    .bind(body.idempotency_key)
    .bind(serde_json::to_value(&response).map_err(|e| format!("encode error: {e}"))?)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("database error: {e}"))?;

    tx.commit()
        .await
        .map_err(|e| format!("database error: {e}"))?;

    Ok(response)
}

/// Canonical SHA-256 hash of a receive-stock request, for idempotency replay.
///
/// Includes `actor` and does not collapse `None`/`Some("")` on `unit_cost`,
/// matching the Postgres storage of `NULL` and `''`.
fn receive_request_hash(body: &ReceiveStockRequest) -> String {
    #[derive(Serialize)]
    struct CanonicalReceive {
        v: &'static str,
        op: &'static str,
        key: String,
        actor: String,
        sku: String,
        location_code: String,
        qty: String,
        unit_cost: Option<String>,
    }

    let canonical = CanonicalReceive {
        v: "1",
        op: RECEIVE_STOCK_OPERATION,
        key: body.idempotency_key.to_string(),
        actor: body.actor.clone(),
        sku: body.sku.clone(),
        location_code: body.location_code.clone(),
        qty: body.qty.clone(),
        unit_cost: body.unit_cost.clone(),
    };

    let bytes = serde_json::to_vec(&canonical).expect("canonical receive request serializes");
    ol_ledger::sha256_hex(&bytes)
}

/// Stable SHA-256 hash of the receipt's inputs, for the audit trail.
///
/// Reuses the same canonical struct as [`receive_request_hash`]: the audit digest
/// must be boundary-unambiguous (`None` vs `Some("")` differ) and versioned so a
/// future format change is explicit. The canonical JSON includes `key` and
/// `actor` because the frozen oracle pins the full `receive_request_hash` form;
/// keeping the two hashes identical prevents the audit trail from ever silently
/// diverging from the idempotency claim that guarded the same receipt.
fn receipt_inputs_hash(body: &ReceiveStockRequest) -> String {
    receive_request_hash(body)
}

/// Receive stock into a location.
#[utoipa::path(
    post,
    path = "/inventory/receive",
    request_body = ReceiveStockRequest,
    responses(
        (status = 201, description = "Stock move created", body = ReceiveStockResponse),
        (status = 200, description = "Idempotency key already used; the original move is returned with `replayed: true`", body = ReceiveStockResponse),
        (status = 404, description = "Item or location not found", body = ErrorBody),
        (status = 422, description = "Invalid numeric quantity or unit cost", body = ErrorBody),
        (status = 500, description = "Database error", body = ErrorBody),
    )
)]
pub async fn receive_stock(
    State(pool): State<PgPool>,
    Json(body): Json<ReceiveStockRequest>,
) -> impl IntoResponse {
    match receive_stock_core(&pool, body).await {
        // 200 on replay, 201 on create: a replay created nothing, and this is the
        // convention `POST /journal-entries` already follows.
        Ok(resp) if resp.replayed => (StatusCode::OK, Json(resp)).into_response(),
        Ok(resp) => (StatusCode::CREATED, Json(resp)).into_response(),
        Err(msg) => {
            let (status, code) = if msg.contains("IDEMPOTENCY_KEY_REUSED") {
                (StatusCode::CONFLICT, ErrorCode::DuplicateIdempotencyKey)
            } else if msg.contains("not found") {
                (StatusCode::NOT_FOUND, ErrorCode::Validation)
            } else if msg.to_lowercase().contains("invalid input syntax")
                || msg.to_lowercase().contains("numeric")
            {
                (StatusCode::UNPROCESSABLE_ENTITY, ErrorCode::Validation)
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, ErrorCode::Internal)
            };
            err_response(status, code, msg).into_response()
        }
    }
}

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
fn engine_error_response(e: EngineError) -> (StatusCode, Json<ErrorEnvelope>) {
    match e {
        EngineError::ProcessNotFound(name) => err_response(
            StatusCode::NOT_FOUND,
            ErrorCode::Validation,
            format!("process not found: {name}"),
        ),
        EngineError::IllegalTransition {
            from,
            capability,
            available,
        } => err_response(
            StatusCode::CONFLICT,
            ErrorCode::Validation,
            if available.is_empty() {
                format!(
                    "no transition from '{from}' with capability '{capability}' (terminal state)"
                )
            } else {
                format!(
                    "no transition from '{from}' with capability '{capability}'. \
                     Available from '{from}': {avail}",
                    avail = available.join(", ")
                )
            },
        ),
        EngineError::InstanceNotActive(status) => err_response(
            StatusCode::CONFLICT,
            ErrorCode::Validation,
            format!("instance is not active (status: {status})"),
        ),
        EngineError::ConcurrentAdvance => err_response(
            StatusCode::CONFLICT,
            ErrorCode::SerializationFailure,
            "concurrent advance conflict: another caller already advanced this instance",
        ),
        EngineError::IdempotencyKeyReused(msg) => err_response(
            StatusCode::CONFLICT,
            ErrorCode::DuplicateIdempotencyKey,
            msg,
        ),
        EngineError::UnknownRole(role) => err_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::Validation,
            format!("unknown account role: {role}"),
        ),
        EngineError::PostingAmountsRequired {
            capability,
            debit_role,
            required_amount_keys,
            missing,
        } => err_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::Validation,
            format!(
                "posting step '{capability}' needs amounts in integer cents, and must include the keys \
                 [{required_amount_keys}]: key 'amount' = the debit total for role '{debit_role}'\
                 {multi_credit_note}. Missing: [{missing}].",
                multi_credit_note = if required_amount_keys.len() > 1 {
                    ", and one key per credited role, all summing to 'amount'"
                } else {
                    " (the single credited account takes the whole total)"
                },
                required_amount_keys = required_amount_keys.join(", "),
                missing = missing.join(", ")
            ),
        ),
        EngineError::Unbalanced { debit, credit } => err_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::UnbalancedEntry,
            format!("posting amounts unbalanced: debit {debit} != credit {credit}"),
        ),
        EngineError::Ledger(e) => post_error_response(e),
        EngineError::Db(e) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            e.to_string(),
        ),
        EngineError::Io(e) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            e.to_string(),
        ),
        EngineError::ProcessLoad(msg) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            format!("process load error: {msg}"),
        ),
    }
}

/// Start a new process instance.
#[utoipa::path(
    post,
    path = "/instances",
    request_body = StartInstanceRequest,
    responses(
        (status = 201, description = "Instance created", body = InstanceResponse),
        (status = 200, description = "Instance replayed from idempotency key", body = InstanceResponse),
        (status = 404, description = "Process not found", body = ErrorBody),
        (status = 409, description = "Idempotency key reused with a different payload", body = ErrorBody),
        (status = 500, description = "Database or IO error", body = ErrorBody),
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
        (status = 404, description = "Instance not found", body = ErrorBody),
        (status = 409, description = "Illegal transition, inactive instance, or concurrent advance", body = ErrorBody),
        (status = 422, description = "Unknown role or unbalanced amounts", body = ErrorBody),
        (status = 500, description = "Database or IO error", body = ErrorBody),
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
        (status = 500, description = "Database error", body = ErrorBody),
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
        (status = 404, description = "Instance not found", body = ErrorBody),
        (status = 500, description = "Database error", body = ErrorBody),
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
