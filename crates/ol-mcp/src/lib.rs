//! ol-mcp — MCP server exposing OpenLedger's typed, idempotent capabilities.
//!
//! Discrete named capabilities only — NO admin shell, NO raw-SQL `execute` tool.
//! See vault `02 Projects/OpenERP/01 Architecture/api-surface.md`.
//!
//! Auth (ADR-006, MCP spec draft):
//! - OAuth 2.1 + mandatory PKCE (S256) for public clients; no implicit flow.
//! - RFC 9728 Protected Resource Metadata at /.well-known/oauth-protected-resource (MUST).
//! - RFC 8414 AS metadata; RFC 8707 resource indicators (audience-bind, MUST for client).
//! - Client registration: Client ID Metadata Documents (SHOULD) preferred; RFC 7591 DCR a MAY fallback.
//! - 401 -> WWW-Authenticate w/ resource_metadata (MUST); 403 insufficient_scope (SHOULD).
//! - Scoped tokens, least privilege: ledger:read, ledger:post, ar:invoice, ap:bill,
//!   payment:register, inventory:move, party:write, account:write, audit:read.
//!   Publish the namespace in `scopes_supported`.
