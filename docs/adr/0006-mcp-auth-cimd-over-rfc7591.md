# ADR-006 — MCP auth: Client ID Metadata Documents over RFC 7591; 403 is SHOULD

- **Status:** Accepted (2026-05-29) · **NOT YET IMPLEMENTED**

## Context
The MCP authorization draft downgraded RFC 7591 Dynamic Client Registration to
MAY and now prefers Client ID Metadata Documents. An earlier internal spec
overstated `403 insufficient_scope` as MUST; it is SHOULD.

## Decision
- Lead client registration with **Client ID Metadata Documents** (SHOULD); keep
  RFC 7591 DCR as a MAY fallback.
- Implement **`401` + `resource_metadata`** (MUST) and
  **`403 insufficient_scope`** (SHOULD).
- `ol-mcp` implements Protected Resource Metadata (RFC 9728, MUST),
  Authorization Server metadata (RFC 8414), and resource indicators (RFC 8707).

## Consequences
- **This is a design decision, not shipped behaviour.** The API and MCP server
  currently run **unauthenticated**; `ol-auth` exists but is not wired in. See
  [SECURITY.md](../../SECURITY.md) and [ADR-022](0022-honest-security-posture.md).
  Wiring this is the project's top open security issue.
- Do not use Twenty as an RFC 8707 reference implementation (open bug #20296).
