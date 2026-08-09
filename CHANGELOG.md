# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- GitHub Actions CI workflow covering `cargo fmt`, `cargo clippy`, `cargo test` against PostgreSQL 17, `cargo-deny`, `gitleaks`, and a web UI lane.
- Architecture decision records under `docs/adr/`.
- Claude Desktop MCP guide at `docs/claude-desktop.md`.

### Changed

- Project renamed from OpenERP to CoolERP, per ADR-023.

### Fixed

- Dashboard cash KPI no longer sums all asset accounts; it now limits the calculation to the cash account only.
