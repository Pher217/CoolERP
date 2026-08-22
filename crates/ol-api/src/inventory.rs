//! Inventory endpoints — stock on-hand query and the idempotent receive-stock write.
//!
//! GET  /inventory          — list items with computed on-hand quantities.
//! POST /inventory/receive  — receive stock into a location (idempotent via `idempotency_key`).

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use ol_sdk::ErrorCode;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::err_response;

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
        (status = 500, description = "Database error", body = crate::ErrorBody),
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
        (status = 404, description = "Item or location not found", body = crate::ErrorBody),
        (status = 422, description = "Invalid numeric quantity or unit cost", body = crate::ErrorBody),
        (status = 500, description = "Database error", body = crate::ErrorBody),
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
