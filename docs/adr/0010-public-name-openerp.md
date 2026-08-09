# ADR-010 — Public name is "OpenERP" (reverses ADR-001)

- **Status:** ⚠️ **Superseded** by [ADR-023](0023-public-name-is-coolerp.md) (2026-08-05)
- **Date:** 2026-06-01
- **Reverses:** [ADR-001](0001-public-name-openledger.md)

## Context
ADR-001 had moved the public name to "OpenLedger". The operator disliked that
name and wanted the product name to match the repository name.

## Decision
Public product and brand name = **OpenERP**. Drop "OpenLedger" everywhere.

## Rationale (as recorded at the time)
The Odoo SEO and brand-collision risk raised by ADR-001 is real and
acknowledged, but the "AI-native / agent-native ERP" positioning is precisely
what differentiates the project from a decade of Odoo legacy content — Odoo
legacy is human-GUI ERP, this is agent-operated. **The collision is a marketing
problem to manage, not an architectural one.**

## Consequences
- Crate names `ol-*` and the `ol` CLI binary were **intentionally kept**;
  renaming them was judged an invasive, build-unverifiable refactor and deferred.
- **Superseded by [ADR-023](0023-public-name-is-coolerp.md).** On re-examination
  the "marketing problem to manage" was never accompanied by a plan to manage it,
  and a live search showed the collision funnels brand traffic to a competitor.
  Two of this ADR's three stated drivers did not survive review: disliking
  "OpenLedger" is an objection to one alternative rather than to renaming, and
  "name should match repo" is circular when the repo name is itself the variable
  under decision.
