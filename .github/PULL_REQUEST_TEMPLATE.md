## Goal

<!-- What and why. Link the ADR, issue, or discussion. -->

## Changes

<!-- Bulleted by component, e.g. `ol-ledger`, `apps/ol-ui`, `migrations`. -->

## Approach

<!-- Why this implementation over alternatives. -->

## Test Summary

| Layer | Command | Result |
|---|---|---|
| Format | `cargo fmt --all --check` | |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | |
| Unit / integration | `cargo test --all` | |
| Web UI | `cd apps/ol-ui && npm run lint && npm test` | |

## Files Changed

<!-- High-level list of files touched. -->

## Checklist

- [ ] Tests added or updated
- [ ] `cargo fmt --all --check` clean
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` clean
- [ ] `cargo test --all` green
- [ ] DCO sign-off present (`git commit -s`)
- [ ] No secrets or `.env` committed
- [ ] ADR added or updated if an architectural decision changed
