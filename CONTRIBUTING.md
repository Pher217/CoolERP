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

> Note on licensing: the core is **AGPL-3.0**; the SDK is **MIT OR Apache-2.0**.
> The DCO certifies that you have the right to submit your contribution under the
> project's existing license — it does **not** grant the maintainer any right to
> relicense your contribution. It is not a CLA.
>
> The project may later offer a commercially-licensed edition and a hosted service,
> which would require the maintainer to hold relicensing rights. If and when that
> path is taken, a **Contributor License Agreement (CLA) will be introduced first**,
> and it would apply to contributions accepted from that point on. Until a CLA is in
> place, all contributions are inbound under AGPL-3.0 (SDK: MIT OR Apache-2.0) via the
> DCO only.

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
