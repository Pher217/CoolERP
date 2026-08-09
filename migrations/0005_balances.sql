-- CoolERP — materialized per-(account, currency) balance cache.
--
-- account_balance() previously summed journal_lines on every read — O(n) per
-- account, the scale cliff every serious ledger (TigerBeetle/Formance/Modern
-- Treasury) materializes past. This cache makes balance reads O(1) and
-- per-currency-correct, maintained INSIDE the posting transaction by a trigger
-- on journal_lines so it can never be written out of band.
--
-- The cache is DERIVED state; journal_lines remain the source of truth. Because
-- journal_lines is append-only (UPDATE/DELETE forbidden by 0001), an incremental
-- += can never drift from line edits — there are none. A reconciliation test
-- asserts the cache equals a fresh SUM. Money stays integer cents. (ADR-015)

CREATE TABLE account_balances (
    account_id BIGINT NOT NULL REFERENCES accounts (id),
    currency   TEXT   NOT NULL,
    debits     BIGINT NOT NULL DEFAULT 0,
    credits    BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, currency)
);

-- Maintain the cache on every line insert, in the same transaction. Concurrent
-- posts to the same (account, currency) serialize on this row — already the case
-- via the posting engine's FOR UPDATE on the accounts row, so no new contention.
CREATE OR REPLACE FUNCTION bump_account_balance() RETURNS trigger AS $$
BEGIN
    INSERT INTO account_balances (account_id, currency, debits, credits)
    VALUES (NEW.account_id, NEW.currency, NEW.debit, NEW.credit)
    ON CONFLICT (account_id, currency) DO UPDATE
        SET debits  = account_balances.debits  + EXCLUDED.debits,
            credits = account_balances.credits + EXCLUDED.credits;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER trg_bump_balance
    AFTER INSERT ON journal_lines
    FOR EACH ROW EXECUTE FUNCTION bump_account_balance();

-- Backfill from existing lines (writes the cache directly; the trigger above
-- fires only on journal_lines inserts, not on this).
INSERT INTO account_balances (account_id, currency, debits, credits)
SELECT account_id, currency, SUM(debit), SUM(credit)
  FROM journal_lines
 GROUP BY account_id, currency;
