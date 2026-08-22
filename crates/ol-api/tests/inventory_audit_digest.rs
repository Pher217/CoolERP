//! Audit-digest pinning for inventory receipts.
//!
//! `events.inputs_hash` for a stock receipt must be a stable SHA-256 digest of
//! the receipt's economic content. Today `receipt_inputs_hash` uses
//! `std::collections::hash_map::DefaultHasher`, which is neither stable across
//! processes nor a cryptographic digest, so this test is expected to fail until
//! the audit hash is switched to the same SHA-256 canonical scheme already used
//! by `receive_request_hash` for idempotency.

use ol_api::{ReceiveStockRequest, receive_stock_core};
use sqlx::PgPool;
use uuid::Uuid;

/// Pinned SHA-256 test vector for a specific `ReceiveStockRequest`.
///
/// Computed from the canonical JSON `receive_request_hash` uses:
/// `{"v":"1","op":"receive_stock","key":"11111111-2222-3333-4444-555555555555",
/// "actor":"test","sku":"WIDGET-A","location_code":"MAIN","qty":"10",
/// "unit_cost":"250"}`.
const PINNED_DIGEST: &str = "4c59f137ea2f42a5eaf34546128b1e1a82d8b0d8093070ab9d6451d7e20734f7";

fn receipt(
    key: Uuid,
    sku: &str,
    location_code: &str,
    qty: &str,
    unit_cost: Option<&str>,
) -> ReceiveStockRequest {
    ReceiveStockRequest {
        idempotency_key: key,
        sku: sku.to_string(),
        location_code: location_code.to_string(),
        qty: qty.to_string(),
        unit_cost: unit_cost.map(String::from),
        actor: "test".to_string(),
    }
}

async fn inputs_hash_for_move(pool: &PgPool, move_id: i64) -> String {
    sqlx::query_scalar(
        "SELECT inputs_hash FROM events WHERE capability = 'receive_stock' AND entity_id = $1",
    )
    .bind(move_id.to_string())
    .fetch_one(pool)
    .await
    .expect("event must exist for move")
}

/// GIVEN inventory receipts whose audit digests must be stable and
/// boundary-unambiguous,
/// WHEN each receipt is recorded,
/// THEN (1) a fixed request produces the pinned SHA-256 digest,
/// (2) a boundary shift between sku and location_code produces a different
/// digest, and (3) `unit_cost: None` and `unit_cost: Some("")` produce
/// different digests.
#[sqlx::test(migrations = "../../migrations")]
async fn inventory_receipt_audit_digest_is_pinned_sha256(pool: PgPool) {
    // ─── Outcome 1: pinned SHA-256 vector ─────────────────────────────────────
    let pinned_key = Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap();
    let pinned = receive_stock_core(
        &pool,
        receipt(pinned_key, "WIDGET-A", "MAIN", "10", Some("250")),
    )
    .await
    .expect("pinned receipt must succeed");

    let pinned_hash = inputs_hash_for_move(&pool, pinned.move_id).await;
    assert_eq!(
        pinned_hash, PINNED_DIGEST,
        "audit digest must match the pinned SHA-256 vector"
    );

    // ─── Outcome 2: boundary shift must change the digest ───────────────────
    sqlx::query("INSERT INTO items (sku, name) VALUES ('AB', 'AB item'), ('A', 'A item')")
        .execute(&pool)
        .await
        .expect("seed boundary items");
    sqlx::query("INSERT INTO locations (code, name) VALUES ('C', 'C loc'), ('BC', 'BC loc')")
        .execute(&pool)
        .await
        .expect("seed boundary locations");

    let boundary_ab_c = receive_stock_core(&pool, receipt(Uuid::new_v4(), "AB", "C", "1", None))
        .await
        .expect("AB/C receipt");
    let boundary_a_bc = receive_stock_core(&pool, receipt(Uuid::new_v4(), "A", "BC", "1", None))
        .await
        .expect("A/BC receipt");

    let ab_c_hash = inputs_hash_for_move(&pool, boundary_ab_c.move_id).await;
    let a_bc_hash = inputs_hash_for_move(&pool, boundary_a_bc.move_id).await;
    assert_ne!(
        ab_c_hash, a_bc_hash,
        "boundary shift between sku and location_code must produce a different digest"
    );

    // ─── Outcome 3: None vs Some("") unit_cost must differ ──────────────────
    let none_cost = receive_stock_core(
        &pool,
        receipt(Uuid::new_v4(), "WIDGET-A", "MAIN", "1", None),
    )
    .await
    .expect("None unit_cost receipt");
    let empty_cost = receive_stock_core(
        &pool,
        receipt(Uuid::new_v4(), "WIDGET-A", "MAIN", "1", Some("")),
    )
    .await
    .expect("empty unit_cost receipt");

    let none_hash = inputs_hash_for_move(&pool, none_cost.move_id).await;
    let empty_hash = inputs_hash_for_move(&pool, empty_cost.move_id).await;
    assert_ne!(
        none_hash, empty_hash,
        "None and Some(\"\") unit_cost must produce different audit digests"
    );
}
