-- OpenLedger v0.1 — minimal chart of accounts seed + general journal.
-- ON CONFLICT DO NOTHING on both tables so this migration is idempotent and
-- coexists with test fixtures that insert the same rows (e.g. posting.rs seed()).

INSERT INTO journals (code, name) VALUES ('GEN', 'General')
ON CONFLICT (code) DO NOTHING;

INSERT INTO accounts (code, name, type) VALUES
    ('1000', 'Cash',                 'asset'),
    ('1100', 'Accounts Receivable',  'asset'),
    ('1200', 'Inventory',            'asset'),
    ('2000', 'Accounts Payable',     'liability'),
    ('2100', 'Tax Payable',          'liability'),
    ('3000', 'Owner Equity',         'equity'),
    ('4000', 'Sales Revenue',        'income'),
    ('5000', 'COGS',                 'expense')
ON CONFLICT (code) DO NOTHING;
