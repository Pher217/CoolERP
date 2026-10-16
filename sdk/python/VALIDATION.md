# Validation - 2026-10-14

Local verification used Python 3.12.9, Rust 1.99.0, and disposable PostgreSQL
17.11 (Debian bookworm), starting from CoolERP 7944e3f. No production ledger
or production endpoint was used. The SDK is an unpublished prototype.

| Check | Result |
|---|---|
| Actual router OpenAPI + Tungsten 0.1.2 strict generation check | PASS; 17 operations / 27 files; TG0303 and TG0760 remain informational |
| Installed wheel with published runtime 0.1.2 | PASS: 336 tests in 2.43s on PostgreSQL 17; 329 offline + 7 live |
| Monetary validation | All 20 declared cents fields, exact >2^53 values, i64 bounds, invalid input types, strict sync/async responses |
| Journal and balance | 201 balanced GEN entry; 200 UUID replay, no double count; 409 changed payload; 422 imbalance; 404 unknown account |
| `pip check` | PASS, no broken requirements |
| `cargo fmt --all --check` | PASS |
| `cargo clippy --locked --all-targets --all-features -- -D warnings` | PASS |
| `cargo test --locked --all` | PASS; one pre-existing ignored inputs-hash test (#53) |
| `sqlx migrate run` | PASS: all nine migrations into a separate empty disposable database |
| `cargo sqlx prepare --workspace` then `--check --workspace` | PASS; three query records added under `.sqlx` |
| `cargo deny check` (0.20.2) | FAIL: one existing unsafe-libyaml unmaintained advisory; bans/licenses/sources pass |
| Wheel and sdist | PASS: wheel built from sdist; license files, typing marker and overlay receipt included |

The expected Pydantic warning comes from a test deliberately corrupting a model
instance; the invalid request is rejected before network I/O. Live coverage is
limited to journal posting/account balances. Async behavior is tested offline.
UI and full-history secret scans were not run locally; hosted workflow results
must be checked separately. The new Python workflow has not yet run on GitHub.

## Focused prerequisite fixes

Cargo.lock changes only rustls 0.23.40 -> 0.23.46, its webpki 0.103.13 ->
0.103.15 dependency, and chacha20 0.10.0 -> 0.10.2. This clears four TLS
advisories and the yanked-version error. Downloaded package manifests and build
scripts were reviewed before compilation. There are no advisory suppressions.

`advisory-findings.json` retains the six baseline findings for comparison.
The remaining failure is RUSTSEC-2024-0449, unsafe-libyaml 0.2.11 via
serde_yaml_ng 0.10.0 -> ol-process. No newer serde_yaml_ng release is available.
Replacing the parser is a separate prerequisite: it changes accepted YAML,
scalar typing and public error types, requiring focused compatibility tests.
This draft must not be described as having all CI gates green.

The SQLx records cover the three inventory queries reported by the default
workspace prepare command. They are not a claim of offline `--all-targets`
coverage. CI now checks them against migrated PostgreSQL 17.

## Reproduction

See README.md for generation, clean PyPI installation and SDK tests. From the
repository root, the Compose project starts only a disposable local database:

```powershell
docker compose -f sdk/python/compose.yaml run --rm build sh -c 'rustup component add rustfmt clippy && cargo fmt --all --check && cargo clippy --locked --all-targets --all-features -- -D warnings && cargo test --locked --all'
docker compose -f sdk/python/compose.yaml run --rm build sh -c 'cargo install --locked sqlx-cli --version 0.8.6 --no-default-features --features postgres,rustls && cargo sqlx prepare --check --workspace'
docker compose -f sdk/python/compose.yaml run --rm build sh -c 'cargo install --locked cargo-deny --version 0.20.2 && cargo deny check'
```

For a fresh SQLx-managed database (distinct from the entrypoint-migrated SDK DB):

```powershell
docker compose -f sdk/python/compose.yaml run --rm build sh -c 'cargo install --locked sqlx-cli --version 0.8.6 --no-default-features --features postgres,rustls && export DATABASE_URL=postgres://sdk_test:disposable_only@db/sdk_migrations && sqlx database create && sqlx migrate run && cargo sqlx prepare --check --workspace'
```

Runtime code, generator entrypoints, exporter, ledger fixtures and build hooks
were reviewed before local use. This was targeted review, not a full dependency
security audit. Docker/network access used the environment approval mechanism.

## Installation evidence

Fresh public-index verification used `.venv-clean-sdk-012` and Python isolated
mode (`-I`). The runtime was not preinstalled: installing the local SDK wheel
resolved it from PyPI with `--isolated --no-cache-dir --index-url https://pypi.org/simple`.
All third-party artifacts came from files.pythonhosted.org. Runtime wheel SHA256:
`33e4fa2050b480798ea2bcf65b544d3b97ecea79469769fc834108c9fc378f25`.
See `PYPI_VALIDATION.json` and the tested `constraints-py312.txt` snapshot.


- Compiler Windows archive SHA256 (matched release SHA256SUMS):
  `bf91e47254ce7e0d7bad7f5521e48c33910db2a46797f2c6407656c79e9714b7`.
  Archive: `tungsten-0.1.2-x86_64-pc-windows-msvc.zip`. Executable reports 0.1.2.
  This verifies the release checksum; no independent build attestation was checked.
- Captured OpenAPI SHA256:
  `56fb5cb55d542b6cdfaad6cba6b04ba8e42a61b5d98850e0ef0466b2805d5bb5`.
- Current verification uses published `tungsten-runtime` 0.1.2 from PyPI in a
  fresh environment. `direct_url.json` is absent; metadata remains Apache-2.0.
  The earlier local source runtime is no longer used for verification.
- Installed SDK version: 0.1.0.dev0; httpx 0.28.1, Pydantic 2.14.1,
  pydantic-core 2.50.1, pytest 9.1.1; build 1.6.1 / hatchling 1.32.4.
- Wheel metadata: `Requires-Python: >=3.12`; runtime, Pydantic, HTTPX ranges
  match the generation manifest (runtime >=0.1.2,<0.2.0); `License-Expression: MIT OR Apache-2.0`.
- Wheel and sdist both contain LICENSE-MIT, LICENSE-APACHE,
  `coolerp_sdk/py.typed`, and strict cents extensions (under `generated/` in the
  sdist). The wheel also carries `_money_overlay.json`; sdist carries the build
  hook and overlay source. Runtime is a separate dependency, not bundled.
- The earlier PyPI check failed before publication. Prince subsequently published
  runtime 0.1.2; clean installation now succeeds. See `PYPI_VALIDATION.json`.
  This task did not publish any package.
