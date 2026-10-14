# Contribution requirements reviewed at 7944e3f

## Explicit requirements

- `CONTRIBUTING.md`: every contributed commit needs the contributor's DCO 1.1
  sign-off; unsigned PR commits will not be merged. This certifies rights to
  submit under existing licensing. It is not a CLA and grants no relicensing
  rights. No current CLA is required; the document says one would be introduced
  before future contributions needing relicensing rights. The contributor must personally approve
  the DCO and use the approved public identity for commit sign-offs.
- ADR-003, amended by ADR-009: core/server/API/MCP/UI/CLI are AGPL-3.0; Python
  and TypeScript SDKs are generated from OpenAPI and use MIT OR Apache-2.0.
  The exporter is a core example; the Python package uses existing SDK licenses.
- `AGENTS.md`, ADR-007 and `CONTRIBUTING.md`: integer cents throughout, no floats;
  append-only ledger; every entry balances; preserve idempotency; no raw SQL MCP
  execution. Changes to ledger logic require property tests; this prototype
  does not change ledger logic. Property testing remains required before release.
- Pre-PR checks: `cargo fmt --all --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all`
  with local Postgres, and new dependencies passing `cargo deny check`.
  AGENTS additionally lists `sqlx migrate run`,
  `cargo sqlx prepare --check --workspace`, and `cargo deny check licenses`.
- Commit convention: `feat:`, `fix:`, `refactor:`, `docs:`, `test:`, or `chore:`;
  no AI attribution in commit messages or git history. No blanket prohibition
  on AI-assisted contributions, separate disclosure form, or AI-specific legal
  agreement was found in these reviewed files. This is not permission to omit
  truthful answers if maintainers later ask about the work.
- Contributor Covenant 2.1: respectful, harassment-free participation; no
  disclosure of private information; applies in community spaces and official
  representation. Enforcement ranges from correction to permanent ban. The
  repository lists a private reporting contact; no contact was made.

## Templates, CI, and recommendations

- PR template requests Goal, Changes, Approach, Test Summary, Files Changed;
  checklist includes tests, fmt, clippy, workspace tests, DCO, no secrets/.env,
  and an ADR when an architectural decision changes. Keep PRs focused and give
  goal/change/test evidence (`CONTRIBUTING.md`). This contribution is intended for draft review.
- Feature template asks problem, solution, alternatives, context. Bug template
  asks reproduction, Rust/Postgres versions, and whether money/balances are
  affected. Blank issues are disabled. Security reports go to private GitHub
  advisories. These are submission formats, not additional local build gates.
- CI runs Rust fmt/clippy/tests against migrated Postgres 17; cargo-deny;
  gitleaks on history with redaction; and UI npm ci/lint/test/build using Node 22.
  This change adds Python SDK CI; there is no explicit DCO job in the existing workflow;
  absence of that job does not waive the DCO policy.
- Local validation now uses PostgreSQL 17.11. No UI changes were made, and UI
  checks were not run locally. Rust lockfile patch updates address TLS advisories
  and a yanked cipher dependency. The YAML parser migration needs a separate
  prerequisite review. Do not represent cargo-deny as a Python dependency audit.

## Submission scope

Draft PR review is appropriate with the remaining cargo-deny failure stated
explicitly. No package release, deployment, or merge is part of this change.
The SDK uses the published tungsten-runtime 0.1.2 package from PyPI.
A maintained YAML parser replacement should be reviewed separately because it
changes core process parsing semantics, including scalar typing and errors.
