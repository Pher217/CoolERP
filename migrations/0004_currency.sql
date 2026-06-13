-- OpenERP — multi-currency at the journal-line level (per-currency double-entry).
--
-- Before this, journal_lines had no currency and the balance triggers summed
-- debits/credits GLOBALLY. A "balanced" entry could mix a USD debit with an EUR
-- credit of equal integer value and pass — silently destroying value (gotcha #17,
-- flagged by Modern Treasury). Now every entry must balance WITHIN EACH CURRENCY.
--
-- Pre-FX, a genuine cross-currency entry cannot balance per currency without a
-- rate, so it is correctly REJECTED rather than recorded wrong. FX gain/loss
-- handling (a functional-currency amount per line) is a later PR; this migration
-- only makes single-currency entries safe and cross-currency entries impossible.
-- Money stays integer cents. (ADR-007, ADR-014)

ALTER TABLE journal_lines ADD COLUMN currency TEXT NOT NULL DEFAULT 'EUR';
CREATE INDEX idx_journal_lines_currency ON journal_lines (entry_id, currency);

-- ===========================================================================
-- INVARIANT 1 (revised) — >= 2 lines, and within EVERY currency Σdebit = Σcredit.
-- Replaces the global-sum bodies from 0001 (trigger objects are unchanged; only
-- the function bodies are redefined).
-- ===========================================================================
CREATE OR REPLACE FUNCTION assert_entry_balanced() RETURNS trigger AS $$
DECLARE
    v_entry BIGINT := COALESCE(NEW.entry_id, OLD.entry_id);
    v_lines INT;
    v_bad   INT;
BEGIN
    SELECT COUNT(*) INTO v_lines FROM journal_lines WHERE entry_id = v_entry;

    -- Entry may have been fully removed within the tx — nothing to check.
    IF v_lines = 0 THEN
        RETURN NULL;
    END IF;
    IF v_lines < 2 THEN
        RAISE EXCEPTION 'UNBALANCED_ENTRY: entry % has % line(s), need >= 2', v_entry, v_lines;
    END IF;

    SELECT COUNT(*) INTO v_bad FROM (
        SELECT currency
          FROM journal_lines
         WHERE entry_id = v_entry
         GROUP BY currency
        HAVING SUM(debit) <> SUM(credit)
    ) unbalanced_currencies;

    IF v_bad > 0 THEN
        RAISE EXCEPTION 'UNBALANCED_ENTRY: entry % does not balance within each currency', v_entry;
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

-- INVARIANT 1b (revised) — same per-currency check, fired from the entry header.
CREATE OR REPLACE FUNCTION assert_entry_has_balanced_lines() RETURNS trigger AS $$
DECLARE
    v_lines INT;
    v_bad   INT;
BEGIN
    SELECT COUNT(*) INTO v_lines FROM journal_lines WHERE entry_id = NEW.id;

    IF v_lines < 2 THEN
        RAISE EXCEPTION 'UNBALANCED_ENTRY: entry % has % line(s), need >= 2', NEW.id, v_lines;
    END IF;

    SELECT COUNT(*) INTO v_bad FROM (
        SELECT currency
          FROM journal_lines
         WHERE entry_id = NEW.id
         GROUP BY currency
        HAVING SUM(debit) <> SUM(credit)
    ) unbalanced_currencies;

    IF v_bad > 0 THEN
        RAISE EXCEPTION 'UNBALANCED_ENTRY: entry % does not balance within each currency', NEW.id;
    END IF;
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;
