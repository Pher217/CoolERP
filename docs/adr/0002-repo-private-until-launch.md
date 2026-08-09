# ADR-002 — Repo private until launch

- **Status:** Accepted (2026-05-29) · superseded in practice once the repo goes public
- **Deciders:** Philippe Hermann

## Context
CoolERP's credibility surface is accounting correctness. Publishing a ledger
project before its double-entry invariants are property-tested invites the one
class of first impression the project cannot recover from: a public general
ledger that loses money.

## Decision
Create the repository **private**. Do not publish publicly until the
double-entry invariant is property-tested green.

## Consequences
- Repo created with `--private`; the public-launch playbook is deferred.
- Flipping visibility is an explicit, separate operator decision.
