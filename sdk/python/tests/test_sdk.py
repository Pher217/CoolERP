"""Offline contract tests; MockTransport cannot contact a real ledger."""
import asyncio
import json
from decimal import Decimal
from uuid import uuid4

import httpx
import pytest
from pydantic import ValidationError
from tungsten_runtime import ClientOptions, Err, Ok

from coolerp_sdk import AsyncCoolerpClient, CoolerpClient
from coolerp_sdk.custom.money import LedgerLine
from coolerp_sdk.models.coolerp import LineRequest


def entry(key=None, amount=10000):
    return dict(actor="sdk-disposable-test", entry_date="2025-01-15",
                journal_code="GEN", idempotency_key=key or str(uuid4()),
                lines=[LedgerLine(account_code="1000", debit=amount, credit=0),
                       LedgerLine(account_code="4000", debit=0, credit=amount)])


@pytest.mark.parametrize("bad", [1.0, 1.5, True, False, "10000", Decimal("10000"),
                                  -1, 2**63, float("nan"), float("inf")])
def test_strict_cents_reject_invalid_input(bad):
    with pytest.raises(ValidationError):
        LedgerLine(account_code="1000", debit=bad, credit=0)


@pytest.mark.parametrize("debit,credit", [(0, 0), (1, 1)])
def test_strict_line_rejects_invalid_sides(debit, credit):
    with pytest.raises(ValidationError):
        LedgerLine(account_code="1000", debit=debit, credit=credit)


def test_packaged_raw_model_closes_generator_gaps():
    # No LedgerLine helper: normal generated model entry points are protected.
    for value in [10000.0, 2**63]:
        with pytest.raises(ValidationError):
            LineRequest(account_code="1000", debit=value, credit=0)


def test_exact_large_cents_wire_and_balance():
    amount = 2**53 + 1  # Detect any accidental IEEE-754 round trip.
    seen = []

    def handle(request):
        seen.append(request)
        if request.method == "POST":
            body = json.loads(request.content)
            assert body["lines"][0]["debit"] == amount
            assert type(body["lines"][0]["debit"]) is int
            assert "Idempotency-Key" not in request.headers
            return httpx.Response(201, json=dict(entry_id=1, balanced=True, replayed=False))
        assert request.url.path == "/accounts/1000/balance"
        return httpx.Response(200, json=dict(account_code="1000", currency="EUR",
                                            debits=amount, credits=0, balance=amount))

    with CoolerpClient(ClientOptions(transport=httpx.MockTransport(handle)),
                       validate_responses="strict") as client:
        assert isinstance(client.journal_entries.create(**entry(amount=amount)), Ok)
        balance = client.accounts.balance.get(code="1000")
        assert isinstance(balance, Ok)
        assert type(balance.value.balance) is int
        assert balance.value.balance == amount
    assert len(seen) == 2


def test_bad_uuid_never_reaches_transport():
    def forbidden(request):
        pytest.fail("invalid UUID reached transport")
    with CoolerpClient(ClientOptions(transport=httpx.MockTransport(forbidden))) as client:
        result = client.journal_entries.create(**entry(key="not-a-uuid"))
        assert isinstance(result, Err)


def test_timeout_does_not_retry_mutation():
    seen = []
    def timeout(request):
        seen.append(json.loads(request.content)["idempotency_key"])
        raise httpx.ReadTimeout("simulated unknown outcome", request=request)
    key = str(uuid4())
    with CoolerpClient(ClientOptions(transport=httpx.MockTransport(timeout))) as client:
        result = client.journal_entries.create(**entry(key=key))
        assert isinstance(result, Err)
    assert seen == [key]


def test_async_balanced_post_and_balance():
    seen = []
    async def handle(request):
        seen.append(request.url.path)
        if request.method == "POST":
            assert json.loads(request.content)["lines"][0]["debit"] == 10000
            return httpx.Response(201, json=dict(entry_id=1, balanced=True, replayed=False))
        return httpx.Response(200, json=dict(account_code="1000", currency="EUR",
                                            debits=10000, credits=0, balance=10000))
    async def run():
        async with AsyncCoolerpClient(ClientOptions(transport=httpx.MockTransport(handle)),
                                      validate_responses="strict") as client:
            assert isinstance(await client.journal_entries.create(**entry()), Ok)
            result = await client.accounts.balance.get(code="1000")
            assert isinstance(result, Ok)
            assert type(result.value.balance) is int and result.value.balance == 10000
    asyncio.run(run())
    assert seen == ["/journal-entries", "/accounts/1000/balance"]
