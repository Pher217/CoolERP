"""Public models, flattened requests and response parsing must all enforce cents."""
import asyncio
import json
from decimal import Decimal
from importlib.resources import files

import httpx
import pytest
from pydantic import ValidationError
from tungsten_runtime import ClientOptions, Err, Ok, UNSET

from coolerp_sdk import AsyncCoolerpClient, CoolerpClient
from coolerp_sdk.models import coolerp as models

FIELDS = {
    "AdvanceInstanceRequest": ["amounts"],
    "AgingBucket": ["total_cents"],
    "BalanceResponse": ["balance", "credits", "debits"],
    "LineRequest": ["credit", "debit"],
    "OverviewResponse": ["ap_cents", "ar_cents", "cash_cents", "expenses_cents", "revenue_cents"],
    "SubledgerSection": ["control_balance_cents", "difference_cents", "subledger_total_cents"],
    "TrialBalanceLine": ["balance_cents", "credits_cents", "debits_cents"],
    "TrialBalanceResponse": ["total_credits_cents", "total_debits_cents"],
}
CASES = [(model, field) for model, fields_ in FIELDS.items() for field in fields_]
FIXTURES = {
    "AdvanceInstanceRequest": dict(capability="post_invoice"),
    "AgingBucket": dict(label="current", total_cents=0),
    "BalanceResponse": dict(account_code="1000", currency="EUR", balance=0, credits=0, debits=0),
    "LineRequest": dict(account_code="1000", debit=0, credit=0),
    "OverviewResponse": dict(account_count=2, ap_cents=0, ar_cents=0, cash_cents=0,
                             expenses_cents=0, revenue_cents=0),
    "SubledgerSection": dict(control_balance_cents=0,
                             subledger_total_cents=0, difference_cents=None,
                             subledger_implemented=False),
    "TrialBalanceLine": dict(account_type="asset", code="1000", name="Cash", currency="EUR",
                             balance_cents=0, credits_cents=0, debits_cents=0),
    "TrialBalanceResponse": dict(balanced=True, lines=[], total_credits_cents=0, total_debits_cents=0),
}


def payload(model, field, value):
    return FIXTURES[model] | {field: {"total": value} if field == "amounts" else value}


@pytest.mark.parametrize("model,field", CASES)
@pytest.mark.parametrize("bad", [1.0, 1.5, True, False, "1", Decimal("1"),
                                  -(2**63)-1, 2**63, float("nan"), float("inf")])
def test_all_public_money_model_fields_reject_invalid(model, field, bad):
    cls = getattr(models, model)
    data = payload(model, field, bad)
    with pytest.raises(ValidationError):
        cls(**data)
    if not isinstance(bad, Decimal):
        with pytest.raises(ValidationError):
            cls.model_validate_json(json.dumps(data))


@pytest.mark.parametrize("model,field", CASES)
@pytest.mark.parametrize("amount", [-(2**63), -1, 0, 2**53+1, 2**63-1])
def test_all_public_money_fields_preserve_exact_signed_i64(model, field, amount):
    cls = getattr(models, model)
    data = payload(model, field, amount)
    for parsed in [cls(**data), cls.model_validate_json(json.dumps(data))]:
        value = getattr(parsed, field)
        value = value["total"] if field == "amounts" else value
        assert type(value) is int and value == amount
        wire = json.loads(parsed.model_dump_json())
        assert wire[field] == data[field]


def test_optional_nullable_and_non_money_semantics_remain():
    assert models.AdvanceInstanceRequest(capability="post").amounts is UNSET
    assert models.SubledgerSection(**FIXTURES["SubledgerSection"]).difference_cents is None
    assert models.PostResultResponse(entry_id=1.0, balanced=True, replayed=False).entry_id == 1
    # This separate endpoint explicitly accepts decimal strings, not i64 cents.
    data = dict(actor="test", idempotency_key="550e8400-e29b-41d4-a716-446655440001",
                sku="test", location_code="test", qty="1", unit_cost="1.2345")
    assert models.ReceiveStockRequest(**data).unit_cost == "1.2345"
    for bad in [1.0, True, 1, Decimal("1")]:
        with pytest.raises(ValidationError):
            models.ReceiveStockRequest(**(data | {"unit_cost": bad}))


@pytest.mark.parametrize("bad", [1.0, True, 2**63, -(2**63)-1])
def test_flattened_amounts_and_raw_line_dicts_rejected_before_io(bad):
    def forbidden(request):
        pytest.fail("invalid request reached transport")
    with CoolerpClient(ClientOptions(transport=httpx.MockTransport(forbidden))) as client:
        assert isinstance(client.instances.advance(id=1, capability="post", amounts={"total": bad}), Err)
        assert isinstance(client.journal_entries.create(
            actor="test", journal_code="GEN", entry_date="2025-01-15",
            idempotency_key="550e8400-e29b-41d4-a716-446655440001",
            lines=[dict(account_code="1000", debit=bad, credit=0)]), Err)


def test_mutated_or_constructed_model_revalidated_before_io():
    def forbidden(request):
        pytest.fail("invalid model instance reached transport")
    mutated = models.LineRequest(account_code="1000", debit=1, credit=0)
    mutated.debit = 1.0
    bypassed = models.LineRequest.model_construct(account_code="1000", debit=2**63, credit=0)
    with CoolerpClient(ClientOptions(transport=httpx.MockTransport(forbidden))) as client:
        for line in [mutated, bypassed]:
            assert isinstance(client.journal_entries.create(
                actor="test", journal_code="GEN", entry_date="2025-01-15",
                idempotency_key="550e8400-e29b-41d4-a716-446655440001", lines=[line]), Err)


@pytest.mark.parametrize("bad", [1.0, True, 2**63, -(2**63)-1])
def test_invalid_server_money_is_error_by_default_sync_and_async(bad):
    response = FIXTURES["BalanceResponse"] | {"balance": bad}
    def handle(request):
        return httpx.Response(200, json=response)
    with CoolerpClient(ClientOptions(transport=httpx.MockTransport(handle))) as client:
        assert isinstance(client.accounts.balance.get(code="1000"), Err)
    async def run():
        async with AsyncCoolerpClient(ClientOptions(transport=httpx.MockTransport(handle))) as client:
            assert isinstance(await client.accounts.balance.get(code="1000"), Err)
    asyncio.run(run())


def test_explicit_response_validation_opt_out_remains_available():
    def handle(request):
        return httpx.Response(200, json=FIXTURES["BalanceResponse"] | {"balance": 1.0})
    with CoolerpClient(ClientOptions(transport=httpx.MockTransport(handle)), validate_responses="off") as client:
        result = client.accounts.balance.get(code="1000")
        assert isinstance(result, Ok) and type(result.value["balance"]) is float


def test_packaged_overlay_receipt_covers_reviewed_fields():
    receipt = json.loads(files("coolerp_sdk").joinpath("_money_overlay.json").read_text())
    assert receipt["monetary_fields"] == sorted(f"{model}.{field}" for model, field in CASES)
