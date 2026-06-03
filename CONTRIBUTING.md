# Contributing to OpenERP

Thank you for your interest. OpenERP is an agent-native, double-entry-correct ERP where **accounting correctness is the entire credibility surface** — contributions are held to a high bar.

## Developer Certificate of Origin (DCO)

All contributions require a DCO sign-off. By signing off you certify the
[Developer Certificate of Origin 1.1](https://developercertificate.org/). Add a
`Signed-off-by` line to every commit:

```
Signed-off-by: Your Name <your.email@example.com>
```

Use `git commit -s` to add it automatically. PRs with unsigned commits will not be merged.

> Note on licensing: the core is **AGPL-3.0**; the SDK is **MIT OR Apache-2.0**. The
> project may offer a commercially-licensed edition and a hosted service, which
> requires the maintainer to hold the rights to relicense. The DCO grants the
> inbound rights needed for this; a CLA may be introduced before the project
> accepts substantial external contributions.

## Before you open a PR
- `cargo fmt --all --check` and `cargo clippy --all-targets --all-features -- -D warnings` are clean.
- `cargo test --all` passes; DB-backed tests pass against a local Postgres.
- New dependencies pass `cargo deny check`.
- Changes touching the ledger include property tests for the double-entry invariant.
- No floats for money. No `UPDATE`/`DELETE` on the ledger. No raw-SQL MCP tool.

## Commits & PRs
- Conventional prefixes: `feat: fix: refactor: docs: test: chore:`.
- No AI attribution in commit messages or git history.
- Keep PRs focused; describe goal, changes, and test evidence.
