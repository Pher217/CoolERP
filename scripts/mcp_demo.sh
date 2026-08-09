#!/bin/bash
# CoolERP — agent-native ledger walkthrough over MCP (stdio).
# Every call below is exactly what an AI agent issues; nothing is faked.
set -u
BIN="${OL_MCP_BIN:-./target/debug/ol-mcp}"

say()  { printf '\n\033[1;33m%s\033[0m\n' "$*"; }
note() { printf '\033[2m%s\033[0m\n' "$*"; }
sl()   { sleep "${1:-0.9}"; }

# One MCP call: initialize, then the tool call. State lives in Postgres,
# so a fresh server process per call is equivalent to a long-lived session.
mcp() {
  local tool="$1" args="$2"
  printf '\033[1;36m→ %s\033[0m %s\n' "$tool" "$args"
  {
    echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"demo","version":"0"}}}'
    echo '{"jsonrpc":"2.0","method":"notifications/initialized"}'
    sleep 0.35
    printf '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"%s","arguments":%s}}\n' "$tool" "$args"
    sleep 1.0
  } | "$BIN" 2>/dev/null | python3 -c '
import json,sys
for line in sys.stdin:
    r=json.loads(line)
    if r.get("id")!=2: continue
    res=r.get("result",{})
    txt=(res.get("content") or [{}])[0].get("text","")
    if res.get("isError"):
        print("\033[1;31m  x refused:\033[0m "+txt); break
    try: o=json.loads(txt)
    except Exception:
        print("  "+txt); break
    inst = o.get("instance", o)
    if "current_state" in inst:
        print("  state: \033[1;32m%s\033[0m   (instance %s)" % (inst["current_state"], inst.get("id")))
    for a in o.get("available", []):
        p=a.get("posting")
        post=""
        if p:
            cr = ", ".join(p.get("credit_roles") or [])
            post = "   \033[35m[posts: DR %s%s]\033[0m" % (
                p.get("debit_role"), (" / CR " + cr) if cr else "")
        print("    - \033[1m%-20s\033[0m -> %-16s%s" % (a["capability"], a["to_state"], post))
    break
'
}

clear
say "CoolERP — an AI agent operating a double-entry ledger over MCP"
note "Eight typed tools. The engine tells the agent what is legal; the database"
note "refuses anything that would unbalance the books. No screen-scraping."
sl 2.5

say "1. Start an order-to-cash process"
REF="SO-$(date +%H%M%S)"
mcp start_process "{\"process\":\"order_to_cash\",\"reference\":\"$REF\",\"context\":{}}"
ID=$(psql "$DATABASE_URL" -tAc "select id from process_instances where reference='$REF'")
sl 1.6

say "2. Ask the engine what is legal from here"
note "This is the whole point: the agent does not guess and get rejected."
mcp get_process_instance "{\"instance_id\":$ID}"
sl 2.2

say "3. Try an ILLEGAL move — post the invoice before shipping"
note "A chat-bot-on-an-ERP would attempt this. The engine refuses, and says why."
mcp advance_process "{\"instance_id\":$ID,\"capability\":\"post_invoice\",\"context_patch\":{},\"amounts\":{}}"
sl 2.6

say "4. Walk the legal path instead"
for cap in confirm_order run_credit_check release_credit_hold allocate_inventory; do
  mcp advance_process "{\"instance_id\":$ID,\"capability\":\"$cap\",\"context_patch\":{},\"amounts\":{}}"
  sl 0.7
done
sl 1.2

say "5. The next step posts to the general ledger"
mcp get_process_instance "{\"instance_id\":$ID}"
note "The [posts …] annotation comes from the YAML, not from the model."
sl 2.4

say "6. Deliver — this writes a real journal entry (COGS / inventory)"
mcp advance_process "{\"instance_id\":$ID,\"capability\":\"deliver\",\"context_patch\":{},\"amounts\":{\"amount\":250000}}"
sl 1.8

say "7. The books, straight from the ledger"
psql "$DATABASE_URL" -c "select a.code, a.name, b.currency, b.debits as debits_cents, b.credits as credits_cents from account_balances b join accounts a on a.id=b.account_id order by a.code;"
sl 2.0

say "Every step above is append-only and audited."
note "The balance invariant is enforced by database triggers — not by the agent,"
note "and not by the API layer. The LLM proposes; the deterministic core disposes."
sl 2.5
