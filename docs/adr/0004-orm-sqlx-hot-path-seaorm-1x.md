# ADR-004 — ORM: SQLx on the hot path, SeaORM 1.1.x for CRUD (not SeaORM 2.0)

- **Status:** Accepted (2026-05-29)

## Context
The ledger posting path needs explicit transactions, compile-time-checked SQL,
and predictable performance. The surrounding domain is schema-heavy CRUD where an
ORM earns its keep. An earlier draft assumed "SeaORM 2.0 shipped Jan 2026" — that
was wrong: the referenced post is an RC migration guide. Latest was
`2.0.0-rc.38`; stable was `1.1.20`.

## Decision
Hybrid. **SQLx** for the ledger-posting hot path (raw, compile-time checked,
explicit transactions). **SeaORM pinned to 1.1.x stable** for the CRUD domain —
explicitly *not* 2.0, which is still RC.

## Consequences
- `Cargo.toml` pins `sea-orm = "1.1"`. Revisit when 2.0 stable ships.
- **SQLx pinned to `0.8`, not `0.9`**: SeaORM 1.1.x depends on sqlx 0.8, and a
  verification build showed `sqlx = "0.9"` pulls a *second* copy of sqlx into the
  tree. Aligning the hot path to 0.8 unifies the dependency graph, and 0.8 has the
  macros, `prepare --check`, and transaction support the project needs.
- Because the `query!` macros are compile-time checked, a build needs either a
  live `DATABASE_URL` or a committed `.sqlx` offline cache. CI uses the former.
