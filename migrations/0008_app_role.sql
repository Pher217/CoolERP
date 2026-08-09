-- 0008_app_role.sql — least-privilege application role (defense the triggers cannot provide).
--
-- The append-only triggers in 0001/0007 (BEFORE UPDATE OR DELETE … forbid_mutation())
-- are necessary but NOT sufficient on their own, because:
--   (1) TRUNCATE does not fire row-level BEFORE UPDATE/DELETE triggers — a TRUNCATE
--       would wipe the ledger without tripping a single append-only guard;
--   (2) a table OWNER can `ALTER TABLE … DISABLE TRIGGER` and then mutate freely.
--
-- Both holes close the same way: the application must connect as a role that
--   - owns nothing (cannot DISABLE TRIGGER), and
--   - is never granted UPDATE/DELETE on the append-only tables, nor TRUNCATE on anything.
--
-- `ol_app` is that role. Deployments create a LOGIN user and `GRANT ol_app TO <user>`;
-- the app's DATABASE_URL uses that user. Migrations continue to run as the schema
-- OWNER (a separate, privileged role), so DDL is unaffected. See ADR-021.

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'ol_app') THEN
        CREATE ROLE ol_app NOLOGIN;
    END IF;
END
$$;

GRANT USAGE ON SCHEMA public TO ol_app;

-- Read + append on every existing table; usage on sequences for the append inserts.
GRANT SELECT, INSERT ON ALL TABLES IN SCHEMA public TO ol_app;
GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO ol_app;

-- Narrow UPDATE: only the genuinely-mutable state tables the app writes.
--   idempotency_keys   — result backfill after a first successful op (ol-ledger)
--   process_instances  — guarded state advance (ol-engine)
--   fiscal_periods     — open/close (ol-ledger::periods)
--   account_balances   — cache maintained by bump_account_balance() (INSERT … ON CONFLICT DO UPDATE)
GRANT UPDATE ON idempotency_keys, process_instances, fiscal_periods, account_balances TO ol_app;

-- Belt-and-braces: ol_app is never granted DELETE or TRUNCATE anywhere above, and must
-- never hold UPDATE/DELETE/TRUNCATE on the append-only ledger tables. Revoke explicitly
-- so intent is auditable and a future blanket GRANT cannot silently re-open the hole.
REVOKE UPDATE, DELETE, TRUNCATE ON journal_entries, journal_lines, events, process_steps_log FROM ol_app;

-- Tables/sequences created by future migrations (as the same owner) default to the
-- same least-privilege posture for ol_app.
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT ON TABLES TO ol_app;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT USAGE, SELECT ON SEQUENCES TO ol_app;
