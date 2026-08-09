# ADR-003 — Licensing: AGPL-3.0 core, MIT/Apache SDK, DCO from day one

- **Status:** Accepted (2026-05-29)
- **Amended by:** [ADR-009](0009-rust-core-polyglot-edges.md) (SDK is realized primarily as Python + TypeScript packages)

## Context
The project needs a licence that keeps the core open while leaving a hosted
commercial offering viable, and it needs an inbound-contribution policy in place
*before* the first external contribution arrives — retrofitting one is painful.
Precedents reviewed: Twenty (AGPLv3) and Odoo Community.

## Decision
- Core / server / API / MCP / UI / CLI: **AGPL-3.0**.
- SDK: **MIT OR Apache-2.0**.
- Require a **DCO** sign-off from the first external contribution.

## Consequences
- `LICENSE` (AGPL-3.0) at the repo root; the SDK carries its own dual licence.
- `CONTRIBUTING.md` documents the sign-off requirement.
- **The DCO certifies inbound rights under the existing licence only — it grants
  no relicensing rights.** A CLA is required before any relicensing. See
  [ADR-022](0022-honest-security-posture.md), which corrected an earlier wording
  that implied otherwise.
