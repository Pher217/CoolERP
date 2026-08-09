# ADR-008 — The human view is a web SPA (React 19 + Vite + TypeScript)

- **Status:** Accepted (2026-05-29) · supersedes the earlier Tauri-desktop choice
- **Amended by:** [ADR-009](0009-rust-core-polyglot-edges.md)

## Context
The project needs a human-facing view alongside the agent-facing MCP surface.
Rust/WASM options were evaluated and rejected for now: Leptos 0.8 is in
light-maintenance, Dioxus 0.7 is pre-1.0, both would require building a
virtualized data grid and a bespoke security-critical WASM OAuth client, and
ICU4X has no locale currency formatter ([icu4x#6804](https://github.com/unicode-org/icu4x/issues/6804)).

## Decision
The human view is a browser **web app** (`apps/ol-ui`): **React 19 + Vite +
TypeScript**, consuming the Rust REST API. The API contract is generated from
Axum via utoipa (**OpenAPI 3.1**) and consumed as TypeScript. Each workflow's
human view is a **Mermaid `stateDiagram-v2` generated from `processes/*.yaml`**,
so the YAML stays the single source of truth. Money is formatted with
`Intl.NumberFormat` from integer cents. `apps/ol-ui` is a **separate package, not
in the Cargo workspace**; the built app is served as static files by Axum.

## Consequences
- Adds a JS toolchain and its own CI lane.
- **UI auth is gated on the OAuth Authorization Server from
  [ADR-006](0006-mcp-auth-cimd-over-rfc7591.md), which does not exist yet.** Until
  it lands the UI runs in dev mode.
- Avoid commercial or copyleft frontend dependencies (AG Grid Enterprise,
  MUI X Pro, bpmn-js watermark).
