-- CoolERP v0.1 — seed inventory master data.
-- Inserts one default location and four sample items.  Uses ON CONFLICT DO
-- NOTHING so the migration can be safely re-applied and tests can run on a
-- database that already contains this data.

INSERT INTO locations (code, name)
VALUES ('MAIN', 'Main Warehouse')
ON CONFLICT (code) DO NOTHING;

INSERT INTO items (sku, name)
VALUES
    ('WIDGET-A', 'Widget A'),
    ('WIDGET-B', 'Widget B'),
    ('GADGET-1', 'Gadget One'),
    ('BOLT-10',  'Bolt 10mm')
ON CONFLICT (sku) DO NOTHING;
