# Tungsten 0.1.4 upgrade validation - 2026-10-28

PR #136 remains open and ready for review. Its head was verified as
`fa1d4b0393527c38d05b21945c254ea24ef8daaa` with no conversation, review or inline
comments before this upgrade. The previous task workspace had disappeared;
its exact pushed branch was restored into a separate clone without modifying
other project checkouts.

- Generator tag v0.1.4 resolves to `87786d95db97ed9a8908de4acca66b32bbd25ed0`.
  Windows compiler archive SHA256, checked against release sums and asset digest:
  `360aaa65d18aeafa04b03ea5e7e4a85503eba4dc2fe23659115577ec4f5181c4`.
- Runtime wheel SHA256:
  `eb60f38d0878fb899458d35a3e4b2e6f34a45042f7f45b6a1cc22fbdab70ccc9`.
  All 24 Python files match the reviewed tag source after CRLF normalization.
  Transport and the monetary overlay patterns are unchanged from 0.1.3.
- The only dependency constraint updates are tungsten-runtime 0.1.3 -> 0.1.4
  and certifi 2026.7.22 -> 2026.10.19, required by runtime 0.1.4. No npm
  component is involved; npm release availability is not assumed.
- Strict regeneration and drift check pass for all 17 operations / 27 files.
  TG0303 and TG0760 remain informational. The actual disposable router OpenAPI
  equals the preserved checked-in document. All 20 strict cents fields remain.
- Fresh wheel and sdist installations independently resolve the published
  runtime and certifi from PyPI, with no source runtime override or direct URL.
  Both pass `pip check`. Each passes 331 offline/transport tests and all seven
  disposable PostgreSQL API tests: **338 tests per installation**. One expected
  warning diagnoses a deliberately corrupted monetary model instance.
- Rust formatting, Clippy (all targets/features) and workspace tests pass on
  the unchanged Rust source/lockfile; the baseline ignored ledger test remains.
- All nine migrations pass in a separate empty disposable database;
  `cargo sqlx prepare --check --workspace` passes. No cache records changed.
- cargo-deny still fails on RUSTSEC-2026-0382 (rustls-webpki),
  RUSTSEC-2026-0386 (tokio), RUSTSEC-2024-0449 (unsafe-libyaml), and
  RUSTSEC-2026-0381 (webpki-roots). Bans/licenses/sources pass. No unrelated
  dependency fix or policy suppression is included.

Use README.md for generation and package installation. For both artifact
installations run the suite without the API first, then set
`COOLERP_DISPOSABLE_TEST=1` and run `tests/test_disposable_api.py` only after
starting this task's disposable Compose API. Current package provenance is in
`PYPI_VALIDATION.json`. Earlier sections below are historical evidence only.
Hosted CI must be checked on the pushed upgrade commit; `action_required` is
not a passing result. No package release, deployment or merge is involved.

# Historical Tungsten 0.1.3 validation - 2026-10-25

PR #136 was verified open, ready for review, with no reviews/comments and head
30fd6ed9a998a79e014aa05432abf9e64ced93c2 before this scoped upgrade. Work used a
separate detached worktree; the original checkout was not edited.

- Compiler v0.1.3 source commit: `9a8d60e6437fff409041caab4d3f8d87e067604e`.
- Windows release ZIP SHA256, matched release SHA256SUMS and GitHub asset digest:
  `14dbdfc914df4ed409264e013e407e4e39d17cace8e0102efa41c339dbde647c`.
- Published runtime wheel SHA256:
  `cba61fcaf595b1a75d13ee2e2d60a037de3a6a513a2645b037d1d3079dff15b3`.
  All 24 Python files equal reviewed tag source after line-ending normalization.
- Strict generation and drift check pass: 17 operations, 27 files. TG0303 and
  TG0760 remain informational; no claim of complete schema support.
- All 20 cents overrides remain. Version 0.1.3 still accepts integral floats and
  unbounded ints without this explicit overlay. Python remains >=3.12.
- Runtime synchronous native transport changed; added a real loopback server
  test that stalls a successful POST body beyond its deadline and verifies Err,
  bounded return time and no retry. A second test verifies custom transport
  callbacks execute on the caller's thread. No production endpoint is used.
- Fresh wheel and sdist installs independently resolve runtime 0.1.3 from PyPI,
  including the new sniffio dependency (tested 1.3.1). HTTPX stays at 0.28.1.
- **338 tests pass in each installation**: original 329 offline tests, two new
  transport regressions and seven disposable PostgreSQL 17 API tests.
  One expected warning diagnoses a deliberately corrupted monetary model.
  Both environments pass `pip check`.
- Rust fmt, Clippy (all targets/features), workspace tests, all nine migrations
  into an empty disposable DB, and SQLx workspace cache check pass. The actual
  local router OpenAPI equals the checked-in contract. Rust source/lockfile and
  SQLx cache are unchanged by this upgrade.
- **cargo-deny fails with four current advisories** in the unchanged Rust
  dependency tree: RUSTSEC-2026-0382 (rustls-webpki 0.103.15),
  RUSTSEC-2026-0386 (tokio 1.52.3), RUSTSEC-2024-0449 (unsafe-libyaml 0.2.11),
  and RUSTSEC-2026-0381 (webpki-roots 1.0.7). Bans/licenses/sources pass.
  These are reported separately, not suppressed or silently fixed in this SDK
  upgrade. Historical findings below are not the current advisory result.
- Hosted CI requires a separate exact-head check after push. Previous
  `action_required` workflows are not passing test results. No UI changes or
  local UI checks are part of this upgrade.

Reproduction uses README.md with Tungsten 0.1.3. Build with
`python -m build sdk/python`, then install its wheel and sdist into separate new
venvs using `pip --isolated install --no-cache-dir --index-url https://pypi.org/simple
-c sdk/python/constraints-py312.txt <artifact> pytest`. Against the documented
disposable Compose API, set `COOLERP_DISPOSABLE_TEST=1` and run
`python -I -m pytest sdk/python/tests -q`. The original Rust commands below apply;
use a new disposable database name such as `sdk_migrations_013` for migration
validation. `PYPI_VALIDATION.json` contains current runtime provenance.

The following baseline records describe the earlier 0.1.2 work only.

# Historical baseline validation - 2026-10-14

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
