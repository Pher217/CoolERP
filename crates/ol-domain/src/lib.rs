//! ol-domain — core entities and double-entry invariants for OpenLedger.
//!
//! Money is represented as integer cents (`i64`). No floats. (ADR-007)
//!
//! [`assert_balanced`] mirrors the database-level invariant (a deferred
//! `CONSTRAINT TRIGGER` in `migrations/0001_init.sql`) as defense in depth and
//! as a fast, pure property-test target. The database remains the source of
//! truth — this function must never be the *only* enforcement point.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Integer-cents money amount. Never a float.
pub type Cents = i64;

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
/// non-negative, in integer cents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Line {
    pub account_code: String,
    pub debit: Cents,
    pub credit: Cents,
}

/// Violations of the double-entry invariant. Codes match the MCP error envelope.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LedgerError {
    #[error("UNBALANCED_ENTRY: debits {debits} != credits {credits}")]
    Unbalanced { debits: Cents, credits: Cents },
    #[error("UNBALANCED_ENTRY: an entry needs >= 2 lines, got {0}")]
    TooFewLines(usize),
    #[error("INVALID_LINE: each line must have exactly one of debit/credit non-zero and both >= 0")]
    InvalidLine,
}

/// Assert the double-entry invariant over a proposed set of lines:
/// at least two lines, every line single-sided and non-negative, and
/// `sum(debits) == sum(credits)`.
pub fn assert_balanced(lines: &[Line]) -> Result<(), LedgerError> {
    if lines.len() < 2 {
        return Err(LedgerError::TooFewLines(lines.len()));
    }
    let mut debits: Cents = 0;
    let mut credits: Cents = 0;
    for l in lines {
        // exactly one side non-zero, both non-negative
        if l.debit < 0 || l.credit < 0 || (l.debit == 0) == (l.credit == 0) {
            return Err(LedgerError::InvalidLine);
        }
        debits += l.debit;
        credits += l.credit;
    }
    if debits != credits {
        return Err(LedgerError::Unbalanced { debits, credits });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn line(account: &str, debit: Cents, credit: Cents) -> Line {
        Line { account_code: account.into(), debit, credit }
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
            Err(LedgerError::Unbalanced { debits: 10_000, credits: 9_000 })
        );
    }

    #[test]
    fn single_line_rejected() {
        let lines = vec![line("1000", 10_000, 0)];
        assert_eq!(assert_balanced(&lines), Err(LedgerError::TooFewLines(1)));
    }

    #[test]
    fn double_sided_line_rejected() {
        let lines = vec![line("1000", 10_000, 10_000), line("4000", 0, 10_000)];
        assert_eq!(assert_balanced(&lines), Err(LedgerError::InvalidLine));
    }

    proptest! {
        // For any positive amount split across one debit and one credit of the
        // same value, the entry is balanced.
        #[test]
        fn any_matched_pair_balances(amount in 1..1_000_000_000_i64) {
            let lines = vec![line("1000", amount, 0), line("4000", 0, amount)];
            prop_assert_eq!(assert_balanced(&lines), Ok(()));
        }
    }
}
