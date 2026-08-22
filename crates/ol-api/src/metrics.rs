//! Dashboard metrics endpoints.
//!
//! GET /metrics/overview  — aggregated P&L / balance sheet snapshot.
//! GET /metrics/ar-aging  — accounts-receivable aging buckets.
//!
//! All amounts are integer cents (i64). No floats.

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use ol_sdk::ErrorCode;
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;

use crate::err_response;

// ─── GET /metrics/overview ───────────────────────────────────────────────────

/// Snapshot of key financial metrics, all in integer cents.
#[derive(Debug, Serialize, ToSchema)]
pub struct OverviewResponse {
    /// Balance of the cash account, code `1000` (debit-positive).
    #[schema(value_type = i64, format = Int64)]
    pub cash_cents: i64,
    /// Total balance of all income accounts (credit-positive).
    #[schema(value_type = i64, format = Int64)]
    pub revenue_cents: i64,
    /// Total balance of all expense accounts (debit-positive).
    #[schema(value_type = i64, format = Int64)]
    pub expenses_cents: i64,
    /// Balance of account code '1100' (Accounts Receivable).
    #[schema(value_type = i64, format = Int64)]
    pub ar_cents: i64,
    /// Balance of account code '2000' (Accounts Payable).
    #[schema(value_type = i64, format = Int64)]
    pub ap_cents: i64,
    /// Total number of accounts in the chart of accounts.
    #[schema(value_type = i64, format = Int64)]
    pub account_count: i64,
}

/// Raw row returned by the overview aggregate query.
#[derive(sqlx::FromRow)]
struct OverviewRow {
    cash_cents: i64,
    revenue_cents: i64,
    expenses_cents: i64,
    ar_cents: i64,
    ap_cents: i64,
    account_count: i64,
}

/// Get a snapshot of key financial metrics.
#[utoipa::path(
    get,
    path = "/metrics/overview",
    responses(
        (status = 200, description = "Financial overview snapshot", body = OverviewResponse),
        (status = 500, description = "Database error", body = crate::ErrorBody),
    )
)]
pub async fn get_overview(State(pool): State<PgPool>) -> impl IntoResponse {
    // Single query: aggregate per account-type, plus dedicated rows for AR/AP codes.
    // Type normalisation mirrors the ledger convention:
    //   asset / expense  → balance = debits  – credits  (debit-positive)
    //   liability / equity / income → balance = credits – debits  (credit-positive)
    //
    // We compute the signed balance inside the DB so Rust stays integer-only.
    let result: Result<Option<OverviewRow>, sqlx::Error> = sqlx::query_as(
        r#"
        SELECT
            -- cash: balance of the specific cash account (code '1000'), NOT all
            -- assets -- summing every asset double-counts AR/inventory into "cash".
            COALESCE(SUM(
                CASE WHEN a.code = '1000'
                     THEN COALESCE(b.debits, 0) - COALESCE(b.credits, 0)
                     ELSE 0 END
            ), 0)::bigint AS cash_cents,

            -- revenue: sum of all income balances (credit-positive)
            COALESCE(SUM(
                CASE WHEN a.type = 'income'
                     THEN COALESCE(b.credits, 0) - COALESCE(b.debits, 0)
                     ELSE 0 END
            ), 0)::bigint AS revenue_cents,

            -- expenses: sum of all expense balances (debit-positive)
            COALESCE(SUM(
                CASE WHEN a.type = 'expense'
                     THEN COALESCE(b.debits, 0) - COALESCE(b.credits, 0)
                     ELSE 0 END
            ), 0)::bigint AS expenses_cents,

            -- ar_cents: balance of the specific AR account (code '1100')
            COALESCE(SUM(
                CASE WHEN a.code = '1100'
                     THEN COALESCE(b.debits, 0) - COALESCE(b.credits, 0)
                     ELSE 0 END
            ), 0)::bigint AS ar_cents,

            -- ap_cents: balance of the specific AP account (code '2000')
            COALESCE(SUM(
                CASE WHEN a.code = '2000'
                     THEN COALESCE(b.credits, 0) - COALESCE(b.debits, 0)
                     ELSE 0 END
            ), 0)::bigint AS ap_cents,

            COUNT(a.id)::bigint AS account_count
        FROM accounts a
        LEFT JOIN account_balances b
               ON b.account_id = a.id AND b.currency = a.currency
        "#,
    )
    .fetch_optional(&pool)
    .await;

    match result {
        Ok(Some(r)) => (
            StatusCode::OK,
            Json(OverviewResponse {
                cash_cents: r.cash_cents,
                revenue_cents: r.revenue_cents,
                expenses_cents: r.expenses_cents,
                ar_cents: r.ar_cents,
                ap_cents: r.ap_cents,
                account_count: r.account_count,
            }),
        )
            .into_response(),
        Ok(None) => (
            StatusCode::OK,
            Json(OverviewResponse {
                cash_cents: 0,
                revenue_cents: 0,
                expenses_cents: 0,
                ar_cents: 0,
                ap_cents: 0,
                account_count: 0,
            }),
        )
            .into_response(),
        Err(e) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            format!("database error: {e}"),
        )
        .into_response(),
    }
}

// ─── GET /metrics/ar-aging ────────────────────────────────────────────────────

/// A single AR aging bucket.
#[derive(Debug, Serialize, ToSchema)]
pub struct AgingBucket {
    /// Human-readable bucket label.
    pub label: String,
    /// Total open invoice amount in this bucket, in integer cents.
    #[schema(value_type = i64, format = Int64)]
    pub total_cents: i64,
}

/// AR aging report with 5 buckets.
#[derive(Debug, Serialize, ToSchema)]
pub struct ArAgingResponse {
    pub buckets: Vec<AgingBucket>,
}

/// Get accounts-receivable aging report.
///
/// Buckets cover open invoices (state NOT IN ('paid', 'void')):
/// - current: due_date >= today or null
/// - 1-30: 1–30 days overdue
/// - 31-60: 31–60 days overdue
/// - 61-90: 61–90 days overdue
/// - 90+: more than 90 days overdue
#[utoipa::path(
    get,
    path = "/metrics/ar-aging",
    responses(
        (status = 200, description = "AR aging report", body = ArAgingResponse),
        (status = 500, description = "Database error", body = crate::ErrorBody),
    )
)]
pub async fn get_ar_aging(State(pool): State<PgPool>) -> impl IntoResponse {
    // Query returns one row per bucket label. We map to a fixed ordered list so
    // all five buckets are always present (zero if empty).
    let rows: Result<Vec<(String, i64)>, sqlx::Error> = sqlx::query_as(
        r#"
        SELECT
            CASE
                WHEN due_date IS NULL OR due_date >= CURRENT_DATE          THEN 'current'
                WHEN CURRENT_DATE - due_date BETWEEN 1  AND 30             THEN '1-30'
                WHEN CURRENT_DATE - due_date BETWEEN 31 AND 60             THEN '31-60'
                WHEN CURRENT_DATE - due_date BETWEEN 61 AND 90             THEN '61-90'
                ELSE '90+'
            END AS label,
            COALESCE(SUM(total), 0)::bigint AS total_cents
        FROM invoices
        WHERE state NOT IN ('paid', 'void')
        GROUP BY 1
        "#,
    )
    .fetch_all(&pool)
    .await;

    match rows {
        Err(e) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            format!("database error: {e}"),
        )
        .into_response(),
        Ok(db_rows) => {
            // Fixed bucket order; DB may return a subset (missing = zero).
            const LABELS: &[&str] = &["current", "1-30", "31-60", "61-90", "90+"];
            let buckets = LABELS
                .iter()
                .map(|label| {
                    let total_cents = db_rows
                        .iter()
                        .find(|(l, _)| l.as_str() == *label)
                        .map(|(_, v)| *v)
                        .unwrap_or(0);
                    AgingBucket {
                        label: label.to_string(),
                        total_cents,
                    }
                })
                .collect();

            (StatusCode::OK, Json(ArAgingResponse { buckets })).into_response()
        }
    }
}
