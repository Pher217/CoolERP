# ADR-025 — `PostingRequirement` gains `required_amount_keys`; `credit_roles` becomes descriptive

- **Status:** Accepted (2026-08-09) · **actioned** in [PR #87](https://github.com/Pher217/CoolERP/pull/87)
- **Deciders:** Philippe Hermann

## Context
`PostingRequirement` told callers what a posting transition needs. Two of its fields
shared a naming convention but not their semantics:

- `debit_role` named an **account role**.
- `credit_roles` was a **caller contract** — "the extra `amounts` keys you must send" —
  which merely *happened* to be spelled with role names when there were several, and was
  **empty** for a single-credit rule.

Nothing signalled the switch. That is not a hypothetical cost: a previous session read
`credit_roles` as an account-role list, filed it as a bug, wrote a fix, and was stopped
only by the test pinning the old meaning.

What moved this from taste to necessity is that the emptiness of `credit_roles` for
single-credit rules is the **proven cause** of
[#78](https://github.com/Pher217/CoolERP/issues/78): the illegal-transition hint printed
the *debit* account as the credited account on 4 of the 6 posting transitions. A field
whose emptiness forces another code path to invent a wrong answer is not a naming
preference.

## Decision
Resolve [#73](https://github.com/Pher217/CoolERP/issues/73) by adding a field and changing
the meaning of an existing one, rather than renaming or leaving it:

```rust
pub struct PostingRequirement {
    pub debit_role: String,                // account role debited                     (unchanged)
    pub credit_roles: Vec<String>,         // account roles credited — ALWAYS populated (semantics CHANGED)
    pub required_amount_keys: Vec<String>, // exactly the keys the caller must send     (NEW)
}
```

`credit_roles` becomes purely **descriptive** — what the posting *does*. `required_amount_keys`
is the **prescriptive** half — what the caller must send. A single-credit rule requires only
`["amount"]`.

### Rejected alternatives
- **Leave it.** Withholds the credited account from agents on 4/6 posting transitions while
  the REST API exposes it, and leaves #78 open. An agent-native product should not make the
  agent its least-informed client.
- **Rename to `extra_amount_keys` and stop.** Honest and smaller, but it documents the
  information loss instead of closing it, and spends the breaking change without buying the
  missing data.

## Consequences
- **Breaking change** to the MCP and REST DTOs. Taken now precisely because the repo is
  private and pre-launch (ADR-002).
- The implementation trap, recorded because it is easy to miss: the three
  `"…plus one key per credit role [{credit_roles}]"` messages and `PostingAmountsRequired`
  read `credit_roles` *because* it meant "extra keys". They must switch to
  `required_amount_keys` or they will start demanding a key for single-credit rules the
  engine does not want.
- `test_available_transitions_posting_transition_exposes_posting_requirement`
  (`crates/ol-engine/tests/o2c.rs`) asserted `credit_roles.is_empty()`. Rewriting that
  assertion **is** the decision being applied — not a test being loosened to pass.
- Closes [#73](https://github.com/Pher217/CoolERP/issues/73) and
  [#78](https://github.com/Pher217/CoolERP/issues/78). 137 tests pass (was 131).

## References
- [PR #87](https://github.com/Pher217/CoolERP/pull/87) · [#73 decision comment](https://github.com/Pher217/CoolERP/issues/73#issuecomment-5233679079) · [#78](https://github.com/Pher217/CoolERP/issues/78)
- Cited in code at `crates/ol-engine/src/lib.rs`, `crates/ol-process/src/lib.rs`, `crates/ol-engine/tests/o2c.rs`
