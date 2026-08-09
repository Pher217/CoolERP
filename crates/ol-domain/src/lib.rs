//! ol-domain — core entities and double-entry invariants for CoolERP.
//!
//! Money is represented as integer cents (`i64`). No floats. (ADR-007)
//!
//! [`assert_balanced`] mirrors the database-level invariant (deferred
//! `CONSTRAINT TRIGGER`s in `migrations/`) as defense in depth and as a fast,
//! pure property-test target. The database remains the source of truth — this
//! function must never be the *only* enforcement point.
//!
//! Balancing is **per currency** (ADR-014): every currency in an entry must have
//! `Σdebit == Σcredit`. A cross-currency entry cannot balance per currency
//! without an FX rate, so it is rejected here and by the DB until FX lands.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

/// Integer-cents money amount. Never a float.
pub type Cents = i64;

/// Default currency when a caller does not specify one.
pub const DEFAULT_CURRENCY: &str = "EUR";

/// The five account classes of a standard chart of accounts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountType {
    Asset,
    Liability,
    Equity,
    Income,
    Expense,
}

/// A single journal line. Exactly one of `debit` / `credit` is non-zero, both
/// non-negative, in integer cents, denominated in `currency`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    pub account_code: String,
    pub debit: Cents,
    pub credit: Cents,
    /// ISO-4217 currency code (e.g. "EUR", "USD"). Lines balance per currency.
    pub currency: String,
}

impl Line {
    /// Construct a line in [`DEFAULT_CURRENCY`].
    pub fn new(account_code: impl Into<String>, debit: Cents, credit: Cents) -> Self {
        Self {
            account_code: account_code.into(),
            debit,
            credit,
            currency: DEFAULT_CURRENCY.to_string(),
        }
    }

    /// Construct a line in an explicit currency.
    pub fn in_currency(
        account_code: impl Into<String>,
        debit: Cents,
        credit: Cents,
        currency: impl Into<String>,
    ) -> Self {
        Self {
            account_code: account_code.into(),
            debit,
            credit,
            currency: currency.into(),
        }
    }
}

/// Violations of the double-entry invariant. Codes match the MCP error envelope.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LedgerError {
    #[error("UNBALANCED_ENTRY: currency {currency} debits {debits} != credits {credits}")]
    Unbalanced {
        currency: String,
        debits: Cents,
        credits: Cents,
    },
    #[error("UNBALANCED_ENTRY: an entry needs >= 2 lines, got {0}")]
    TooFewLines(usize),
    #[error("INVALID_LINE: each line must have exactly one of debit/credit non-zero and both >= 0")]
    InvalidLine,
}

/// Assert the double-entry invariant over a proposed set of lines:
/// at least two lines, every line single-sided and non-negative, and
/// `sum(debits) == sum(credits)` **within each currency**.
///
/// The first offending currency (in code order) is reported. A cross-currency
/// entry whose currencies do not each balance is rejected — there is no FX rate
/// to reconcile them, so accepting it would destroy value.
pub fn assert_balanced(lines: &[Line]) -> Result<(), LedgerError> {
    if lines.len() < 2 {
        return Err(LedgerError::TooFewLines(lines.len()));
    }
    // Accumulate per currency in i128 so a set of near-i64::MAX amounts cannot
    // overflow the sums. BTreeMap keeps currency order deterministic so the
    // reported offender is stable. The DB sums in BIGINT and is the source of
    // truth; this pure check stays a faithful mirror at the boundary.
    let mut by_currency: BTreeMap<&str, (i128, i128)> = BTreeMap::new();
    for l in lines {
        // exactly one side non-zero, both non-negative
        if l.debit < 0 || l.credit < 0 || (l.debit == 0) == (l.credit == 0) {
            return Err(LedgerError::InvalidLine);
        }
        let entry = by_currency.entry(l.currency.as_str()).or_insert((0, 0));
        entry.0 += i128::from(l.debit);
        entry.1 += i128::from(l.credit);
    }
    for (currency, (debits, credits)) in &by_currency {
        if debits != credits {
            return Err(LedgerError::Unbalanced {
                currency: (*currency).to_string(),
                debits: (*debits).clamp(Cents::MIN.into(), Cents::MAX.into()) as Cents,
                credits: (*credits).clamp(Cents::MIN.into(), Cents::MAX.into()) as Cents,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn line(account: &str, debit: Cents, credit: Cents) -> Line {
        Line::new(account, debit, credit)
    }

    fn line_ccy(account: &str, debit: Cents, credit: Cents, ccy: &str) -> Line {
        Line::in_currency(account, debit, credit, ccy)
    }

    #[test]
    fn balanced_two_line_entry_ok() {
        let lines = vec![line("1000", 10_000, 0), line("4000", 0, 10_000)];
        assert_eq!(assert_balanced(&lines), Ok(()));
    }

    #[test]
    fn unbalanced_entry_rejected() {
        let lines = vec![line("1000", 10_000, 0), line("4000", 0, 9_000)];
        assert_eq!(
            assert_balanced(&lines),
            Err(LedgerError::Unbalanced {
                currency: "EUR".into(),
                debits: 10_000,
                credits: 9_000
            })
        );
    }

    #[test]
    fn single_line_rejected() {
        let lines = vec![line("1000", 10_000, 0)];
        assert_eq!(assert_balanced(&lines), Err(LedgerError::TooFewLines(1)));
    }

    #[test]
    fn near_max_amounts_do_not_overflow() {
        // Two debits near i64::MAX vs one matching credit would overflow an i64
        // accumulator; the i128 sums must handle it without panicking. This set is
        // unbalanced (2*MAX != MAX), so it must be reported as unbalanced, never
        // wrap to a false "balanced".
        let lines = vec![
            line("1000", i64::MAX, 0),
            line("1001", i64::MAX, 0),
            line("4000", 0, i64::MAX),
        ];
        assert!(matches!(
            assert_balanced(&lines),
            Err(LedgerError::Unbalanced { .. })
        ));
    }

    #[test]
    fn double_sided_line_rejected() {
        let lines = vec![line("1000", 10_000, 10_000), line("4000", 0, 10_000)];
        assert_eq!(assert_balanced(&lines), Err(LedgerError::InvalidLine));
    }

    #[test]
    fn two_currencies_each_balanced_ok() {
        // EUR balances within EUR, USD balances within USD — the whole entry is ok.
        let lines = vec![
            line_ccy("1000", 10_000, 0, "EUR"),
            line_ccy("4000", 0, 10_000, "EUR"),
            line_ccy("1001", 5_000, 0, "USD"),
            line_ccy("4001", 0, 5_000, "USD"),
        ];
        assert_eq!(assert_balanced(&lines), Ok(()));
    }

    #[test]
    fn cross_currency_globally_equal_but_unbalanced_rejected() {
        // 100 USD debit vs 100 EUR credit: globally the integers are equal, but
        // neither currency balances on its own. Must be rejected (no FX rate).
        let lines = vec![
            line_ccy("1000", 10_000, 0, "USD"),
            line_ccy("4000", 0, 10_000, "EUR"),
        ];
        let err = assert_balanced(&lines).unwrap_err();
        assert!(matches!(err, LedgerError::Unbalanced { .. }), "got {err:?}");
    }

    #[test]
    fn one_currency_balanced_other_not_rejected() {
        // EUR balances, USD does not -> rejected, and the offender is USD.
        let lines = vec![
            line_ccy("1000", 10_000, 0, "EUR"),
            line_ccy("4000", 0, 10_000, "EUR"),
            line_ccy("1001", 5_000, 0, "USD"),
            line_ccy("4001", 0, 4_000, "USD"),
        ];
        assert_eq!(
            assert_balanced(&lines),
            Err(LedgerError::Unbalanced {
                currency: "USD".into(),
                debits: 5_000,
                credits: 4_000
            })
        );
    }

    proptest! {
        // For any positive amount split across one debit and one credit of the
        // same value in the same currency, the entry is balanced.
        #[test]
        fn any_matched_pair_balances(amount in 1..1_000_000_000_i64) {
            let lines = vec![line("1000", amount, 0), line("4000", 0, amount)];
            prop_assert_eq!(assert_balanced(&lines), Ok(()));
        }
    }
}
