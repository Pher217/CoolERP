//! ol-api — Axum REST API over the ledger + SeaORM-backed CRUD.
//!
//! Stage 1: read-mostly REST surface (accounts, balances, journal, audit) plus
//! the CRUD domain (parties, items, invoices, bills) via SeaORM 1.1.x. The hot
//! posting path goes through `ol-ledger` (SQLx), not SeaORM. (ADR-004)
