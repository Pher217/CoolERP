-- Migration 0009: add a canonical request-hash column to idempotency_keys.
--
-- The replay path now compares the stored hash with the incoming request.
-- A NULL request_hash (pre-existing rows from before this migration) fails
-- closed: a reused key with no recorded hash is rejected rather than replayed
-- with an unknown payload.
ALTER TABLE idempotency_keys
    ADD COLUMN request_hash TEXT;

COMMENT ON COLUMN idempotency_keys.request_hash IS
    'Canonical SHA-256 hex digest of the request payload. NULL for pre-0009 rows; '
    'replaying such rows is rejected (fail closed). Matches replay when equal.';
