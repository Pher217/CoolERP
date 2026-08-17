//! GL-completeness G4: trial balance and sub-ledger reconciliation reports.
//!
//! GET /reports/trial-balance             — full trial balance across all accounts.
//! GET /reports/subledger-reconciliation  — AR and AP control vs sub-ledger check.
//!
//! All amounts are integer cents (i64). No floats. (ADR-007)

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use ol_sdk::ErrorCode;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use utoipa::ToSchema;

use crate::err_response;

// ─── GET /reports/trial-balance ──────────────────────────────────────────────

/// One line in the trial balance — one account/currency pair with raw
/// debits/credits and a type-normalised signed balance.
#[derive(Debug, Serialize, ToSchema)]
pub struct TrialBalanceLine {
    /// Account code (e.g. "1000").
    pub code: String,
    /// Account name.
    pub name: String,
    /// Account type as a string (asset, liability, equity, income, expense).
    pub account_type: String,
    /// ISO-4217 currency code for this line (e.g. "USD", "EUR").
    pub currency: String,
    /// Total raw debits posted to this account in this currency, in integer cents.
    #[schema(value_type = i64, format = Int64)]
    pub debits_cents: i64,
    /// Total raw credits posted to this account in this currency, in integer cents.
    #[schema(value_type = i64, format = Int64)]
    pub credits_cents: i64,
    /// Type-normalised signed balance in integer cents.
    /// asset/expense → debits − credits (debit-positive).
    /// liability/equity/income → credits − debits (credit-positive).
    #[schema(value_type = i64, format = Int64)]
    pub balance_cents: i64,
}

/// Trial balance report.
#[derive(Debug, Serialize, ToSchema)]
pub struct TrialBalanceResponse {
    pub lines: Vec<TrialBalanceLine>,
    /// Grand total of raw debits across all accounts, in integer cents.
    #[schema(value_type = i64, format = Int64)]
    pub total_debits_cents: i64,
    /// Grand total of raw credits across all accounts, in integer cents.
    #[schema(value_type = i64, format = Int64)]
    pub total_credits_cents: i64,
    /// True when total_debits_cents == total_credits_cents (ledger is in balance).
    pub balanced: bool,
}

/// Optional query parameters for the trial balance.
#[derive(Debug, Deserialize)]
pub struct TrialBalanceParams {
    /// ISO-4217 currency code filter. When absent, all currencies are summed.
    pub currency: Option<String>,
}

/// Get the trial balance for all accounts.
///
/// Each line is keyed by (account, currency). When `?currency=` is supplied only
/// lines for that currency are returned. When absent, lines for every currency
/// are returned — one line per (account, currency) pair — so amounts in different
/// currencies are never mixed on a single line. The grand `total_debits_cents` /
/// `total_credits_cents` / `balanced` are computed across all returned lines.
/// Lines are ordered by account code then currency.
#[utoipa::path(
    get,
    path = "/reports/trial-balance",
    params(
        ("currency" = Option<String>, Query, description = "ISO-4217 currency filter (optional)")
    ),
    responses(
        (status = 200, description = "Trial balance", body = TrialBalanceResponse),
        (status = 500, description = "Database error", body = crate::ErrorBody),
    )
)]
pub async fn trial_balance(
    State(pool): State<PgPool>,
    Query(params): Query<TrialBalanceParams>,
) -> impl IntoResponse {
    // Raw row: (code, name, type_text, currency, debits, credits)
    // Per-currency grouping is always applied so amounts in different currencies
    // are never summed on a single line (ADR-007 / multi-currency correctness).
    type TrialRow = (String, String, String, String, i64, i64);
    let rows: Result<Vec<TrialRow>, sqlx::Error> = if let Some(ref cur) = params.currency {
        sqlx::query_as(
            r#"
            SELECT
                a.code,
                a.name,
                a.type::text,
                b.currency,
                COALESCE(SUM(b.debits),  0)::bigint AS debits_cents,
                COALESCE(SUM(b.credits), 0)::bigint AS credits_cents
            FROM accounts a
            JOIN account_balances b
              ON b.account_id = a.id AND b.currency = $1
            GROUP BY a.id, a.code, a.name, a.type, b.currency
            ORDER BY a.code, b.currency
            "#,
        )
        .bind(cur)
        .fetch_all(&pool)
        .await
    } else {
        sqlx::query_as(
            r#"
            SELECT
                a.code,
                a.name,
                a.type::text,
                b.currency,
                COALESCE(SUM(b.debits),  0)::bigint AS debits_cents,
                COALESCE(SUM(b.credits), 0)::bigint AS credits_cents
            FROM accounts a
            JOIN account_balances b ON b.account_id = a.id
            GROUP BY a.id, a.code, a.name, a.type, b.currency
            ORDER BY a.code, b.currency
            "#,
        )
        .fetch_all(&pool)
        .await
    };

    match rows {
        Err(e) => err_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::Internal,
            format!("database error: {e}"),
        )
        .into_response(),
        Ok(db_rows) => {
            let mut total_debits: i64 = 0;
            let mut total_credits: i64 = 0;

            let lines: Vec<TrialBalanceLine> = db_rows
                .into_iter()
                .map(|(code, name, acct_type, currency, debits, credits)| {
                    total_debits = total_debits.saturating_add(debits);
                    total_credits = total_credits.saturating_add(credits);

                    let balance = match acct_type.as_str() {
                        "asset" | "expense" => debits - credits,
                        _ => credits - debits,
                    };

                    TrialBalanceLine {
                        code,
                        name,
                        account_type: acct_type,
                        currency,
                        debits_cents: debits,
                        credits_cents: credits,
                        balance_cents: balance,
                    }
                })
                .collect();

            let balanced = total_debits == total_credits;

            (
                StatusCode::OK,
                Json(TrialBalanceResponse {
                    lines,
                    total_debits_cents: total_debits,
                    total_credits_cents: total_credits,
                    balanced,
                }),
            )
                .into_response()
        }
    }
}

// ─── GET /reports/subledger-reconciliation ───────────────────────────────────

/// No production writer exists for invoices / bills yet (#85). Flip this to
/// true in the same change that lands the AR/AP write path — the tests below
/// pin both branches, so flipping it without implementing the writer fails CI.
const SUBLEDGER_WRITE_PATH_EXISTS: bool = false;

/// The raw reconciliation arithmetic. Pure and always meaningful in isolation,
/// so it stays under test even while the reported value is gated below.
fn difference_cents(control_cents: i64, subledger_cents: i64) -> i64 {
    control_cents - subledger_cents
}

/// The difference as reported to clients: `None` while no write path exists.
///
/// Split from `difference_cents` deliberately. Gating the arithmetic itself
/// would have deleted its test coverage until the AR/AP writer lands, which is
/// exactly when a silent regression would be most expensive.
fn reported_difference(control_cents: i64, subledger_cents: i64) -> Option<i64> {
    SUBLEDGER_WRITE_PATH_EXISTS.then(|| difference_cents(control_cents, subledger_cents))
}

/// Reconciliation figures for one sub-ledger (AR or AP).
#[derive(Debug, Serialize, ToSchema)]
pub struct SubledgerSection {
    /// Balance of the GL control account, in integer cents (type-normalised).
    #[schema(value_type = i64, format = Int64)]
    pub control_balance_cents: i64,
    /// Sum of open document totals in the sub-ledger, in integer cents.
    #[schema(value_type = i64, format = Int64)]
    pub subledger_total_cents: i64,
    /// Whether a production write path exists for this sub-ledger. While false,
    /// the sub-ledger is structurally empty and difference_cents is null: the
    /// comparison is not meaningful, and reporting a difference would fabricate
    /// a reconciliation failure equal to the whole control balance (#85).
    pub subledger_implemented: bool,
    /// Difference: control_balance_cents - subledger_total_cents. Zero means
    /// reconciliation passes. NULL when subledger_implemented is false.
    #[schema(value_type = Option<i64>, format = Int64)]
    pub difference_cents: Option<i64>,
}

/// Sub-ledger reconciliation report: AR and AP control vs sub-ledger check.
#[derive(Debug, Serialize, ToSchema)]
pub struct SubledgerReconciliationResponse {
    /// Accounts Receivable reconciliation (GL account '1100' vs open invoices).
    pub ar: SubledgerSection,
    /// Accounts Payable reconciliation (GL account '2000' vs open bills).
    pub ap: SubledgerSection,
}

/// Get the AR and AP sub-ledger reconciliation report.
///
/// - **AR**: compares the balance of GL account `1100` (Accounts Receivable,
///   asset — debit-positive) against the sum of all open **posted** invoices
///   (`state = 'posted'`). Draft invoices are excluded because they have not
///   yet been posted to the AR control account.
/// - **AP**: compares the balance of GL account `2000` (Accounts Payable,
///   liability — credit-positive) against the sum of all open **posted** bills
///   (`state = 'posted'`). Draft bills are excluded for the same reason.
///
/// `difference_cents` is null while `subledger_implemented` is false, because
/// the invoices / bills tables currently have no production write path. The
/// sub-ledger is therefore structurally empty and reporting a numeric
/// difference would fabricate a reconciliation failure equal to the entire
/// control balance (issue #85).
#[utoipa::path(
    get,
    path = "/reports/subledger-reconciliation",
    responses(
        (status = 200, description = "Sub-ledger reconciliation report", body = SubledgerReconciliationResponse),
        (status = 500, description = "Database error", body = crate::ErrorBody),
    )
)]
pub async fn subledger_reconciliation(State(pool): State<PgPool>) -> impl IntoResponse {
    // ── AR: control = balance of account '1100' (asset: debits − credits) ──
    let ar_control: Result<Option<(i64, i64)>, sqlx::Error> = sqlx::query_as(
        r#"
        SELECT
            COALESCE(SUM(b.debits),  0)::bigint,
            COALESCE(SUM(b.credits), 0)::bigint
        FROM accounts a
        LEFT JOIN account_balances b ON b.account_id = a.id
        WHERE a.code = '1100'
        "#,
    )
    .fetch_optional(&pool)
    .await;

    let (ar_debits, ar_credits) = match ar_control {
        Err(e) => {
            return err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                format!("database error: {e}"),
            )
            .into_response();
        }
        Ok(row) => row.unwrap_or((0, 0)),
    };
    // asset: debit-positive
    let ar_control_balance = ar_debits - ar_credits;

    // ── AR sub-ledger: open posted invoices only ─────────────────────────
    // Draft invoices have not yet been posted to the AR control account, so
    // they must not count toward the subledger total. Only `posted` state
    // invoices have a corresponding GL debit on account 1100.
    let ar_subledger: Result<Option<i64>, sqlx::Error> = sqlx::query_scalar(
        r#"
        SELECT COALESCE(SUM(total), 0)::bigint
        FROM invoices
        WHERE state = 'posted'
        "#,
    )
    .fetch_optional(&pool)
    .await;

    let ar_subledger_total = match ar_subledger {
        Err(e) => {
            return err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                format!("database error: {e}"),
            )
            .into_response();
        }
        Ok(row) => row.unwrap_or(0),
    };

    // ── AP: control = balance of account '2000' (liability: credits − debits) ─
    let ap_control: Result<Option<(i64, i64)>, sqlx::Error> = sqlx::query_as(
        r#"
        SELECT
            COALESCE(SUM(b.debits),  0)::bigint,
            COALESCE(SUM(b.credits), 0)::bigint
        FROM accounts a
        LEFT JOIN account_balances b ON b.account_id = a.id
        WHERE a.code = '2000'
        "#,
    )
    .fetch_optional(&pool)
    .await;

    let (ap_debits, ap_credits) = match ap_control {
        Err(e) => {
            return err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                format!("database error: {e}"),
            )
            .into_response();
        }
        Ok(row) => row.unwrap_or((0, 0)),
    };
    // liability: credit-positive
    let ap_control_balance = ap_credits - ap_debits;

    // ── AP sub-ledger: open posted bills only ────────────────────────────
    // Draft bills have not yet been posted to the AP control account, so they
    // must not count toward the subledger total. Only `posted` state bills have
    // a corresponding GL credit on account 2000.
    let ap_subledger: Result<Option<i64>, sqlx::Error> = sqlx::query_scalar(
        r#"
        SELECT COALESCE(SUM(total), 0)::bigint
        FROM bills
        WHERE state = 'posted'
        "#,
    )
    .fetch_optional(&pool)
    .await;

    let ap_subledger_total = match ap_subledger {
        Err(e) => {
            return err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                ErrorCode::Internal,
                format!("database error: {e}"),
            )
            .into_response();
        }
        Ok(row) => row.unwrap_or(0),
    };

    (
        StatusCode::OK,
        Json(SubledgerReconciliationResponse {
            ar: SubledgerSection {
                control_balance_cents: ar_control_balance,
                subledger_total_cents: ar_subledger_total,
                subledger_implemented: SUBLEDGER_WRITE_PATH_EXISTS,
                difference_cents: reported_difference(ar_control_balance, ar_subledger_total),
            },
            ap: SubledgerSection {
                control_balance_cents: ap_control_balance,
                subledger_total_cents: ap_subledger_total,
                subledger_implemented: SUBLEDGER_WRITE_PATH_EXISTS,
                difference_cents: reported_difference(ap_control_balance, ap_subledger_total),
            },
        }),
    )
        .into_response()
}

#[cfg(test)]
mod reconciliation_tests {
    use super::*;

    /// GIVEN a control balance and a sub-ledger total,
    /// WHEN the difference is computed,
    /// THEN it is control minus sub-ledger.
    #[test]
    fn difference_is_control_minus_subledger() {
        assert_eq!(difference_cents(30_000, 30_000), 0, "reconciled");
        assert_eq!(
            difference_cents(30_000, 25_000),
            5_000,
            "control exceeds subledger"
        );
        assert_eq!(
            difference_cents(25_000, 30_000),
            -5_000,
            "subledger exceeds control"
        );
    }

    /// GIVEN no production write path for the sub-ledger,
    /// WHEN the reported difference is computed,
    /// THEN it is None regardless of the inputs.
    ///
    /// The point of #85: with a non-zero control account and a structurally
    /// empty sub-ledger, a numeric answer here is a fabricated reconciliation
    /// failure equal to the entire balance.
    #[test]
    fn reported_difference_is_absent_while_no_write_path_exists() {
        // Pins the current (false) branch. When the AR/AP write path lands and
        // SUBLEDGER_WRITE_PATH_EXISTS flips, this test fails and must be rewritten
        // to assert Some(..) — that failure is the reminder, and is the point.
        assert_eq!(
            reported_difference(30_000, 0),
            None,
            "must not fabricate a difference"
        );
        assert_eq!(
            reported_difference(0, 0),
            None,
            "absent even when it would be zero"
        );
    }
}
