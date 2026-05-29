# Maintainers

OpenLedger's credibility rests on accounting correctness. Each domain below needs an
accountable owner. Three named maintainers (real people + GitHub handles) are required
**before public launch** — see [ROADMAP.md](ROADMAP.md) Stage 3.

## Current

| Domain | Owner | GitHub | Status |
|---|---|---|---|
| Ledger / accounting-correctness | Philippe Hermann | [@Pher217](https://github.com/Pher217) | founding (interim, all domains) |
| MCP / security | _open seat_ | — | recruiting |
| API / data | _open seat_ | — | recruiting |

> Pre-launch gate: at least one maintainer with real double-entry / ERP-accounting depth
> must own ledger correctness before the project goes public. Shipping something
> accounting-incorrect in a launch demo is fatal to trust.

## Responsibilities

- **Ledger / accounting-correctness** — owns the double-entry invariants (DB triggers +
  `ol-domain`), posting engine semantics, and the property-test suite. Final say on
  anything touching balances.
- **MCP / security** — owns `ol-mcp`, the OAuth 2.1 / PKCE flow, scoped tokens, and the
  capability authorization model.
- **API / data** — owns `ol-api`, the schema/migrations, and SeaORM/SQLx data access.

## How to become a maintainer

Sustained, high-quality contributions in a domain + a track record of careful review.
Open a discussion or reach out via an issue. Contributions require a
[DCO sign-off](CONTRIBUTING.md).
