# ADR-024 — Adopt rmcp 2.2: the malformed-argument envelope change is a conformance gain

- **Status:** Accepted (2026-08-09) · **actioned** in [PR #43](https://github.com/Pher217/CoolERP/pull/43)
- **Deciders:** Philippe Hermann

## Context
Upgrading `rmcp` 1.7.0 → 2.2.0 changes one observable behaviour: malformed tool
arguments move from a JSON-RPC `-32602` protocol error to a tool *result* with
`isError: true`.

Whether that is a regression depends entirely on which MCP revision you are
being correct for, because the spec moved in the same direction:

| | 2025-06-18 | 2025-11-25 |
|---|---|---|
| Protocol errors | Unknown tools · **Invalid arguments** · Server errors | Unknown tools · **Malformed requests** (failing the `CallToolRequest` schema) · Server errors |
| Tool execution errors (`isError: true`) | API failures · Invalid input data · Business logic | API failures · **Input validation errors** · Business logic |

A missing required *tool argument* violates that tool's `inputSchema`, not
`CallToolRequest` — which requires only `params.name`. Under **2025-11-25, the
revision this project targets**, it is therefore an input validation error. So
rmcp 2.2 is the conformant one and 1.7 is not.

Verified live, both binaries over stdio, at both negotiated versions: unknown-tool
stays `-32602` on both (correct either way), the bad-argument envelope differs, and
the message text is identical everywhere. **Neither rmcp version is version-aware** —
each applies one envelope regardless of what was negotiated — so this is a choice of
*which revision to be right for*, not conformant-versus-broken.

A `gpt-5.6-sol` audit returned PROBLEM on the grounds that the server "negotiates
2025-06-18". That premise was false: it was an artifact of the probe's own request.
`ol-mcp` advertises `ProtocolVersion::default()` = `LATEST` = `2025-11-25` on both
rmcp versions.

## Decision
Merge the bump to rmcp 2.2.0. **Accept the envelope change deliberately**, and record
it as a known deviation for any client that negotiates MCP `2025-06-18`.

The spec's own stated reason for the reclassification is that tool execution errors
"contain actionable feedback that language models can use to self-correct" — which is
directly on CoolERP's thesis that the agent should be a first-class client.

## Consequences
- Blast radius is ~zero today: stdio-only transport, private repo (ADR-002), pre-launch,
  Claude Desktop the only documented client, and a dependency bump is trivially revertible.
- rmcp 2.2 already knows `V_2026_07_28`, which 1.7 does not.
- Clients pinned to `2025-06-18` see a different envelope for malformed arguments than
  that revision specifies. Accepted; documented here rather than worked around, because
  making it version-aware would mean patching upstream.
- **Residual, untested on both versions:** cancellation, EOF/shutdown, concurrent requests,
  and sub-property schema equivalence.

## References
- [PR #43](https://github.com/Pher217/CoolERP/pull/43) · [decision comment](https://github.com/Pher217/CoolERP/pull/43#issuecomment-5233655414)
- [MCP 2025-11-25 — tools/error handling](https://modelcontextprotocol.io/specification/2025-11-25/server/tools#error-handling)
- [MCP 2025-06-18 — tools/error handling](https://modelcontextprotocol.io/specification/2025-06-18/server/tools#error-handling)
