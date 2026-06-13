//! Fiscal period management — create, close, reopen, and list periods.
//!
//! Periods are opt-in: a date with no covering period posts freely. Only an
//! explicitly CLOSED period blocks insertion (enforced by the DB trigger
//! `trg_period_open` from `migrations/0003_periods.sql`).

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::PostError;

/// A fiscal period row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Period {
    pub code: String,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub state: String,
}

/// Create a new fiscal period.
///
/// Returns [`PostError::PeriodOverlap`] if the date range overlaps an existing
/// period (SQLSTATE `23P01` exclusion violation) or if `code` is not unique
/// (SQLSTATE `23505`).
pub async fn create_period(
    pool: &PgPool,
    code: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Period, PostError> {
    let row: (String, NaiveDate, NaiveDate, String) = sqlx::query_as(
        "INSERT INTO fiscal_periods (code, start_date, end_date) \
         VALUES ($1, $2, $3) \
         RETURNING code, start_date, end_date, state::text",
    )
    .bind(code)
    .bind(start)
    .bind(end)
    .fetch_one(pool)
    .await
    .map_err(|e| {
        if let Some(db) = e.as_database_error() {
            let sqlstate = db.code().map(|c| c.into_owned());
            let msg = db.message().to_string();
            match sqlstate.as_deref() {
                // GiST exclusion violation (overlapping date range)
                Some("23P01") => {
                    return PostError::PeriodOverlap { message: msg };
                }
                // Unique violation on code
                Some("23505") => {
                    return PostError::PeriodOverlap {
                        message: format!("period code '{code}' already exists"),
                    };
                }
                _ => {}
            }
        }
        PostError::Db(e)
    })?;

    Ok(Period {
        code: row.0,
        start_date: row.1,
        end_date: row.2,
        state: row.3,
    })
}

/// Close a fiscal period by code.
///
/// After closing, any INSERT into `journal_entries` whose `effective_date` falls
/// within this period is rejected by the DB trigger.
///
/// Returns [`PostError::PeriodNotFound`] if no period with that code exists.
pub async fn close_period(pool: &PgPool, code: &str) -> Result<(), PostError> {
    let result = sqlx::query(
        "UPDATE fiscal_periods SET state = 'closed', closed_at = now() WHERE code = $1",
    )
    .bind(code)
    .execute(pool)
    .await
    .map_err(PostError::Db)?;

    if result.rows_affected() == 0 {
        return Err(PostError::PeriodNotFound(code.to_string()));
    }
    Ok(())
}

/// Reopen a previously closed fiscal period.
///
/// Returns [`PostError::PeriodNotFound`] if no period with that code exists.
pub async fn reopen_period(pool: &PgPool, code: &str) -> Result<(), PostError> {
    let result =
        sqlx::query("UPDATE fiscal_periods SET state = 'open', closed_at = NULL WHERE code = $1")
            .bind(code)
            .execute(pool)
            .await
            .map_err(PostError::Db)?;

    if result.rows_affected() == 0 {
        return Err(PostError::PeriodNotFound(code.to_string()));
    }
    Ok(())
}

/// List all fiscal periods, ordered by start_date ascending.
pub async fn list_periods(pool: &PgPool) -> Result<Vec<Period>, PostError> {
    let rows: Vec<(String, NaiveDate, NaiveDate, String)> = sqlx::query_as(
        "SELECT code, start_date, end_date, state::text \
           FROM fiscal_periods \
          ORDER BY start_date",
    )
    .fetch_all(pool)
    .await
    .map_err(PostError::Db)?;

    Ok(rows
        .into_iter()
        .map(|(code, start_date, end_date, state)| Period {
            code,
            start_date,
            end_date,
            state,
        })
        .collect())
}
