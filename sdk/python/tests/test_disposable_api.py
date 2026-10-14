"""Opt-in integration tests, fixed loopback endpoint from compose.yaml only."""
import os
from uuid import uuid4

import httpx
import pytest
from tungsten_runtime import ClientOptions, Err, Ok

from coolerp_sdk import CoolerpClient
from test_sdk import entry

pytestmark = pytest.mark.skipif(os.environ.get("COOLERP_DISPOSABLE_TEST") != "1",
                                reason="start sdk/python/compose.yaml and opt in")


@pytest.fixture
def client():
    # No URL environment variable; prevent accidental use of an existing ledger.
    with CoolerpClient(ClientOptions(transport=httpx.HTTPTransport(trust_env=False)),
                       base_url="http://127.0.0.1:38660", validate_responses="strict") as sdk:
        yield sdk


def balance(client):
    result = client.accounts.balance.get(code="1000")
    assert isinstance(result, Ok), result
    assert type(result.value.balance) is int
    return result.value.balance


def test_balanced_post_balance_and_uuid_replay(client):
    before = balance(client)
    request = entry()
    first = client.journal_entries.create(**request)
    assert isinstance(first, Ok), first
    assert first.meta.status == 201
    assert first.value.balanced and not first.value.replayed
    assert balance(client) == before + 10000
    replay = client.journal_entries.create(**request)
    assert isinstance(replay, Ok), replay
    assert replay.meta.status == 200
    assert replay.value.replayed and replay.value.entry_id == first.value.entry_id
    assert balance(client) == before + 10000
    conflict = client.journal_entries.create(**(request | {"memo": "changed payload"}))
    assert isinstance(conflict, Err), conflict
    assert conflict.error["http_status"] == 409
    assert balance(client) == before + 10000


@pytest.mark.parametrize("debit,credit", [(10000, 9999), (-10000, -10000),
                                          (2**63, 2**63), (1.5, 1.5), (True, True)])
def test_invalid_raw_monetary_requests_do_not_change_balance(client, debit, credit):
    before = balance(client)
    request = dict(actor="sdk-disposable-test", entry_date="2025-01-15",
                   journal_code="GEN", idempotency_key=str(uuid4()),
                   lines=[dict(account_code="1000", debit=debit, credit=0),
                          dict(account_code="4000", debit=0, credit=credit)])
    result = client.journal_entries.create(**request)
    assert isinstance(result, Err), result
    if debit == 10000 and credit == 9999:
        assert result.error["http_status"] == 422
    assert balance(client) == before


def test_unknown_account_is_404(client):
    result = client.accounts.balance.get(code="9999")
    assert isinstance(result, Err), result
    assert result.error["http_status"] == 404
