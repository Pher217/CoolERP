# Reproducible SDK monetary validation overlay

The packaged SDK now enforces strict integer cents across **all 20 monetary
int64 field declarations in the captured OpenAPI contract**, through ordinary
generated model constructors, JSON model parsing, request dictionaries, flattened
process amount arguments, and validated responses. `LedgerLine` is optional
additional accounting validation, not the mechanism protecting raw models.

## Build strategy and provenance

Tungsten 0.1.4's untouched output remains in `generated/`. Its generic `Int`
accepts integral floats and has no int64 bounds. Changing that alias globally
would also alter IDs and counts, so this overlay targets reviewed monetary
fields only. No Tungsten source/runtime/worktree was modified.

`prepare_sdk.py` checks the OpenAPI SHA256 (CRLF normalized to LF), generator headers, full
integer-field inventory, and expected annotation patterns. It copies the raw
package into disposable `build/validated/coolerp_sdk`, then applies four explicit
transformations:

1. Monetary model annotations use `custom.cents.Cents`: Pydantic `StrictInt`
   with signed i64 bounds, retaining each field's optional/nullable/container form.
2. The flattened `advance_instance.amounts` descriptor uses the same type.
   Journal requests already validate their nested generated `LineRequest`.
3. Model instances are revalidated at the request boundary, so assignment and
   `model_construct()` cannot bypass money validation when sending a request.
4. Sync/async clients default to strict response validation, including when
   `ClientOptions` supplies a transport. The constructor's `validate_responses`
   keyword takes precedence. Explicit `"warn"`/`"off"` (or `None` with options
   choosing those modes) remains an intentional opt-out and cannot guarantee
   validated typed money. Requests remain validated in those modes.

Each changed build file has an overlay provenance comment. The wheel includes
`_money_overlay.json` containing the source spec hash, overlay script hash,
field inventory and output hashes. `hatch_build.py` invokes preparation on every
wheel build, including wheel-from-sdist builds. Source generation is still
checked independently with `tungsten generate sdk/python --strict --check`.

Do **not** install `generated/` directly or set PYTHONPATH to it: that intentionally
retained unmodified compiler output does not include the overlay. Install
`sdk/python` or its outer-built wheel. Schema/generator drift stops packaging
pending review rather than silently applying stale replacements.

```powershell
# From repository root; this optional inspection build does not edit generated/:
python sdk/python/prepare_sdk.py
# The normal build applies it automatically:
python -m build sdk/python --outdir ../wheelhouse
# Install the wheel, then run tests against the installed package:
../.venv-clean-sdk-012/Scripts/python -m pip --isolated install --index-url https://pypi.org/simple -c sdk/python/constraints-py312.txt ../wheelhouse/coolerp_sdk-0.1.0.dev0-py3-none-any.whl pytest
../.venv-clean-sdk-012/Scripts/python -m pytest sdk/python/tests -q
```

## Reviewed field inventory

| Schema | Cents fields |
|---|---|
| AdvanceInstanceRequest | amounts (each dictionary value) |
| AgingBucket | total_cents |
| BalanceResponse | balance, credits, debits |
| LineRequest | credit, debit |
| OverviewResponse | ap_cents, ar_cents, cash_cents, expenses_cents, revenue_cents |
| SubledgerSection | control_balance_cents, difference_cents, subledger_total_cents |
| TrialBalanceLine | balance_cents, credits_cents, debits_cents |
| TrialBalanceResponse | total_credits_cents, total_debits_cents |

All accept integers from `-2**63` through `2**63-1` without float conversion,
including `2**53+1`. All reject booleans, strings, Decimal, integral/fractional
floats, NaN, infinity and overflow. Signed boundaries intentionally remain valid
at the schema layer: balances can be negative; journal-line sign, sidedness and
entry balancing remain server/database domain rules. The optional `LedgerLine`
helper adds nonnegative/single-sided construction checks.

Unchanged int64 fields: InstanceResponse.id, OverviewResponse.account_count,
PostResultResponse.entry_id, ReceiveStockResponse.move_id, and StepLogResponse's
entry_id/id/instance_id. This is not a blanket int64 fix for Tungsten.

`ReceiveStockRequest.unit_cost` is an explicit contract exception: an optional
nullable **decimal string**, parsed by PostgreSQL as NUMERIC. It remains a strict
string with no conversion to float or cents; tests preserve fractional strings
and reject numeric/bool/Decimal objects. Redefining it as integer cents would
change that endpoint's contract. Opaque JSON context/payload fields have no
schema-declared monetary types; the SDK cannot infer or validate business-specific
money inside them. This remains a limitation of the API contract, not a hidden
claim that every JSON value is cents.

## Request and response evidence

Tests cover every field through normal public constructors and JSON parsing,
valid i64 min/max, negative/zero values and exact values above 2^53. Transport
tests show invalid raw journal dictionaries and flattened process amounts never
reach HTTP. Mutated/constructed model instances also fail before HTTP. The test
of a deliberately corrupted model emits a Pydantic diagnostic serialization
warning; the actual request is rejected.

Response tests exercise both clients with malformed successful HTTP JSON;
default strict mode returns Err. Existing round-trip tests verify exact integer
responses. Explicit response opt-out is tested and documented, not treated as
safe parsing. Disposable API tests retain POST201, replay200, conflict409,
unbalanced422, unknown-account404 and balance invariants.

## SDK workaround versus upstream fix

This can be fixed locally and reproducibly for the pinned CoolERP contract
without an upstream compiler change. The tradeoff is maintaining a small,
fail-closed build overlay. It is a reasonable prototype route, not proof of
general Tungsten integer-format support. A reusable upstream generator option
for strict native integer inputs and int64 bounds would remove this maintenance
burden, but changing Tungsten or contacting its maintainers is outside this
local task. Before eventual submission, Prince should choose whether to propose
this explicit SDK overlay or first address the reusable generator feature.
