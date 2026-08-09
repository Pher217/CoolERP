# ADR-022 — Honest security posture pre-launch

- **Status:** Accepted (2026-07-16)

## Context
An independent review plus direct verification found that `ol-auth` is a fully
unwired island (never referenced from `ol-api` or `ol-mcp`), CORS is
`permissive()`, and the README claimed "same scoped OAuth" — which was false. A
correctness-branded project that ships false security claims damages its own
credibility surface far more than an honest "not done yet."

## Decision
- `ol-api` binds **`127.0.0.1` by default** (was `0.0.0.0`) with a loud startup
  `WARN` that authentication is absent and `actor` is client-asserted.
- `SECURITY.md` documents the true current posture and pins OAuth wiring
  ([ADR-006](0006-mcp-auth-cimd-over-rfc7591.md)) as the top open issue.
- **Correct the DCO/licensing note**: the DCO certifies inbound rights under the
  *existing* licence only and grants **no relicensing rights**. The wording that
  shipped in `CONTRIBUTING.md` implied otherwise, which is legally wrong. A CLA
  is required before any relicensing and now explicitly gates it.

## Consequences
- Full git history secret-scanned clean with `gitleaks`; `.gitleaks.toml`
  allowlists UUID idempotency keys as a known false-positive class.
- Claims in the README are held to what is actually wired.
