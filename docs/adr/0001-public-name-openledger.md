# ADR-001 — Public name is "OpenLedger", not "OpenERP"

- **Status:** ⚠️ **Superseded** by [ADR-010](0010-public-name-openerp.md) (2026-06-01), which was itself superseded by [ADR-023](0023-public-name-is-coolerp.md) (2026-08-05)
- **Date:** 2026-05-29

## Context
"OpenERP" was Odoo's product name from 2009 until June 2014. Research found low
*legal* risk — Odoo holds the "ODOO" mark — but **high brand and SEO collision**
with a decade of Odoo legacy content indexed under the old name.

## Decision
Keep "OpenERP" as the internal working label; use **OpenLedger** as the public
repo and brand name.

## Consequences
- Crate prefix `ol-` and CLI `ol` derive from "OpenLedger".
- **Reversed** six weeks later by ADR-010, then re-decided in a different
  direction by ADR-023. Retained here so the naming trail is complete: the
  `ol-*` prefix that survives in the codebase originates with this decision.
