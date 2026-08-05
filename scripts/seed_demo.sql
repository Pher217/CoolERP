-- seed_demo.sql — idempotent demo data so the ledger / trial-balance / overview views
-- show something real on first run. Every entry is balanced (the deferred balance trigger
-- enforces it at commit); all amounts are integer cents. Safe to run repeatedly.
--
-- Usage: psql "$DATABASE_URL" -f scripts/seed_demo.sql   (or `make seed`)

DO $$
DECLARE
    v_je     bigint;
    a_cash   bigint := (SELECT id FROM accounts WHERE code = '1000');  -- Cash
    a_ar     bigint := (SELECT id FROM accounts WHERE code = '1100');  -- Accounts Receivable
    a_tax    bigint := (SELECT id FROM accounts WHERE code = '2100');  -- Tax Payable
    a_equity bigint := (SELECT id FROM accounts WHERE code = '3000');  -- Owner Equity
    a_rev    bigint := (SELECT id FROM accounts WHERE code = '4000');  -- Sales Revenue
BEGIN
    IF EXISTS (SELECT 1 FROM journal_entries WHERE reference = 'DEMO-0001') THEN
        RAISE NOTICE 'demo seed already present — skipping';
        RETURN;
    END IF;

    -- 1) Opening balance: fund the company with EUR 500,000.00 of owner equity.
    INSERT INTO journal_entries (journal_id, entry_date, memo, reference, posted_at, effective_date)
    VALUES (1, current_date - 30, 'Opening balance — owner capital', 'DEMO-0001', now(), current_date - 30)
    RETURNING id INTO v_je;
    INSERT INTO journal_lines (entry_id, account_id, debit, credit, currency) VALUES
        (v_je, a_cash,   50000000, 0, 'EUR'),
        (v_je, a_equity, 0, 50000000, 'EUR');

    -- 2) Sale on credit: EUR 10,000.00 revenue + EUR 2,000.00 tax => EUR 12,000.00 receivable.
    INSERT INTO journal_entries (journal_id, entry_date, memo, reference, posted_at, effective_date)
    VALUES (1, current_date - 10, 'Invoice INV-1001 — Acme GmbH', 'DEMO-0002', now(), current_date - 10)
    RETURNING id INTO v_je;
    INSERT INTO journal_lines (entry_id, account_id, debit, credit, currency) VALUES
        (v_je, a_ar,  1200000, 0, 'EUR'),
        (v_je, a_rev, 0, 1000000, 'EUR'),
        (v_je, a_tax, 0,  200000, 'EUR');

    -- 3) Partial cash receipt against the invoice: EUR 7,000.00.
    INSERT INTO journal_entries (journal_id, entry_date, memo, reference, posted_at, effective_date)
    VALUES (1, current_date - 2, 'Payment received — Acme GmbH (partial)', 'DEMO-0003', now(), current_date - 2)
    RETURNING id INTO v_je;
    INSERT INTO journal_lines (entry_id, account_id, debit, credit, currency) VALUES
        (v_je, a_cash, 700000, 0, 'EUR'),
        (v_je, a_ar,   0, 700000, 'EUR');

    RAISE NOTICE 'demo seed applied: 3 balanced entries';
END
$$;
