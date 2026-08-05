#!/usr/bin/env bash
# verify_append_only.sh — adversarial proof that the least-privilege app role (0008)
# cannot mutate the append-only ledger, while legitimate appends/updates still work.
#
# The row-level append-only triggers (0001/0007) do NOT fire on TRUNCATE, and a table
# owner can DISABLE TRIGGER. This script proves the *privilege* layer closes both holes:
# it connects as a throwaway LOGIN member of `ol_app` and asserts the boundary.
#
# Usage: DATABASE_URL=postgres://<owner>@host:port/db ./scripts/verify_append_only.sh
# Requires: the target DB has all migrations applied (incl. 0008); the DATABASE_URL role
# can CREATE ROLE; local pg_hba permits a TCP login for the probe role.
set -euo pipefail

ADMIN="${DATABASE_URL:?set DATABASE_URL to an owner/superuser connection string}"
# Derive a probe connection on the same host/db but as role ol_probe. A password is
# supplied so this works under password auth (Docker/CI) as well as trust/peer (local).
PROBE_PW="probe_$$"
PROBE="$(printf '%s' "$ADMIN" | sed -E "s#://[^@/]*@#://ol_probe:${PROBE_PW}@#")"

cleanup() { psql "$ADMIN" -q -c "DROP ROLE IF EXISTS ol_probe;" >/dev/null 2>&1 || true; }
trap cleanup EXIT

psql "$ADMIN" -q -c "DROP ROLE IF EXISTS ol_probe;" \
               -c "CREATE ROLE ol_probe LOGIN PASSWORD '${PROBE_PW}';" \
               -c "GRANT ol_app TO ol_probe;"

pass=0; fail=0
check() { # <desc> <ok|deny> <sql>
  local out rc
  out=$(psql "$PROBE" -v ON_ERROR_STOP=1 -qtAc "$3" 2>&1) && rc=0 || rc=$?
  if [ "$2" = ok ]; then
    if [ "$rc" -eq 0 ]; then echo "PASS  [allow] $1"; pass=$((pass+1))
    else echo "FAIL  [allow] $1 -> $out"; fail=$((fail+1)); fi
  else
    if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -qi 'permission denied'; then
      echo "PASS  [deny ] $1"; pass=$((pass+1))
    else echo "FAIL  [deny ] $1 -> rc=$rc $out"; fail=$((fail+1)); fi
  fi
}

echo "== legitimate balanced posting must succeed =="
if psql "$PROBE" -v ON_ERROR_STOP=1 -q >/dev/null 2>&1 <<'SQL'
BEGIN;
WITH e AS (
  INSERT INTO journal_entries (journal_id, entry_date, memo, posted_at, effective_date)
  VALUES (1, current_date, 'verify_append_only: probe', now(), current_date)
  RETURNING id
)
INSERT INTO journal_lines (entry_id, account_id, debit, credit, currency)
SELECT e.id, 1, 100, 0, 'EUR' FROM e
UNION ALL SELECT e.id, 2, 0, 100, 'EUR' FROM e;
COMMIT;
SQL
then echo "PASS  [allow] balanced entry + lines committed"; pass=$((pass+1))
else echo "FAIL  [allow] balanced posting"; fail=$((fail+1)); fi

echo "== append-only ledger mutation must be denied =="
check "UPDATE journal_lines"     deny "UPDATE journal_lines SET debit = 999 WHERE id > 0;"
check "DELETE journal_entries"   deny "DELETE FROM journal_entries WHERE id > 0;"
check "UPDATE events"            deny "UPDATE events SET actor = 'x' WHERE id > 0;"
check "TRUNCATE events"          deny "TRUNCATE events;"
check "TRUNCATE journal_entries" deny "TRUNCATE journal_entries CASCADE;"
check "TRUNCATE journal_lines"   deny "TRUNCATE journal_lines CASCADE;"
check "UPDATE process_steps_log" deny "UPDATE process_steps_log SET actor = 'x' WHERE id > 0;"

echo "== legitimate state-table updates must be allowed =="
check "UPDATE process_instances" ok "UPDATE process_instances SET current_state = current_state WHERE false;"
check "UPDATE fiscal_periods"    ok "UPDATE fiscal_periods SET state = state WHERE false;"
check "UPDATE idempotency_keys"  ok "UPDATE idempotency_keys SET result = result WHERE false;"

echo
echo "SUMMARY: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
