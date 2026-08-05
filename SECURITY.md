# Security Policy

OpenERP is **pre-1.0 and under active development.** This document is deliberately
honest about what is and isn't protected today, so no one deploys it on a wrong
assumption. Accounting correctness is enforced at the database level and is strong;
**network authentication is not wired yet.**

## Current posture (read before deploying)

| Area | Status |
|---|---|
| Double-entry balance, append-only ledger, idempotency | ✅ **Enforced in PostgreSQL** (triggers + constraints in `migrations/`). Cannot be bypassed by any client. |
| Least-privilege DB role (`ol_app`) — no UPDATE/DELETE/TRUNCATE on the ledger | ✅ Migration `0008_app_role.sql`; proven by `scripts/verify_append_only.sh`. |
| REST API (`ol-api`) authentication | ❌ **None yet.** Runs unauthenticated. Binds `127.0.0.1` by default and warns loudly at startup. |
| MCP server (`ol-mcp`) authentication | ❌ **None yet** — OAuth 2.1 PKCE is designed (ADR-006) but not implemented. |
| Audit-log `actor` | ⚠️ **Client-asserted** until auth lands — it records who the caller *claims* to be, not a verified identity. |
| CORS | ⚠️ Permissive in dev. |

**Wiring authentication (OAuth 2.1 PKCE, ADR-006) into `ol-api` and `ol-mcp` is the
project's #1 open issue.** Until it ships, treat OpenERP as a single-tenant, trusted-
network / localhost tool.

## Safe deployment guidance (until auth lands)

1. **Do not expose the API/MCP off-host.** Keep the default `BIND_ADDR=127.0.0.1`. If you
   must reach it remotely, put it behind an authenticating reverse proxy on a private
   network — never the public internet.
2. **Run the app under the least-privilege role.** Migrations run as the schema owner;
   the *application* must connect as a login user in the `ol_app` group so a compromised
   app process still cannot rewrite history:
   ```sql
   CREATE ROLE openerp_app LOGIN PASSWORD '…';
   GRANT ol_app TO openerp_app;   -- SELECT/INSERT everywhere, UPDATE only on state tables,
                                  -- never UPDATE/DELETE/TRUNCATE on the ledger
   ```
   Point the app's `DATABASE_URL` at `openerp_app`; keep the owner credentials for
   migrations only. Verify the boundary any time with `make verify`.
3. **Never commit secrets.** History is secret-scanned (`.gitleaks.toml`); keep it that way.

## Reporting a vulnerability

Please report security issues **privately**, not via public issues:

- Use GitHub's **[private vulnerability reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability)**
  (repo → *Security* → *Report a vulnerability*), or
- contact a maintainer listed in [`MAINTAINERS.md`](MAINTAINERS.md).

Expect an initial response within a few business days. Because this is a pre-1.0
project, please allow reasonable time for a fix before any public disclosure.
