"""Exercise runtime transport with mocks and an ephemeral loopback server."""
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from uuid import uuid4

import httpx
from tungsten_runtime import ClientOptions, Err, Ok

from coolerp_sdk import CoolerpClient


def test_native_post_body_deadline_does_not_retry(monkeypatch):
    monkeypatch.setenv("NO_PROXY", "127.0.0.1,localhost")
    monkeypatch.setenv("no_proxy", "127.0.0.1,localhost")
    requests = []
    release = threading.Event()

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            self.rfile.read(int(self.headers["Content-Length"]))
            requests.append(self.path)
            self.send_response(201)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", "100")
            self.end_headers()
            self.wfile.flush()
            release.wait(5)  # No response body until the caller has timed out.

        def log_message(self, *args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        with CoolerpClient(base_url=f"http://127.0.0.1:{server.server_port}") as client:
            started = time.monotonic()
            result = client.journal_entries.create(
                actor="sdk-transport-test", entry_date="2025-01-15",
                journal_code="GEN", idempotency_key=str(uuid4()),
                lines=[dict(account_code="1000", debit=10000, credit=0),
                       dict(account_code="4000", debit=0, credit=10000)],
                opts={"timeout_ms": 500},
            )
            assert isinstance(result, Err)
            assert time.monotonic() - started < 3
            assert requests == ["/journal-entries"]
    finally:
        release.set()
        server.shutdown()
        server.server_close()
        thread.join(timeout=2)


def test_sync_custom_transport_runs_on_calling_thread():
    caller = threading.get_ident()
    callbacks = []

    def handle(request):
        callbacks.append(threading.get_ident())
        return httpx.Response(200, json=dict(account_code="1000", currency="EUR",
                                            debits=10000, credits=0, balance=10000))

    with CoolerpClient(ClientOptions(transport=httpx.MockTransport(handle))) as client:
        assert isinstance(client.accounts.balance.get(code="1000"), Ok)
    assert callbacks == [caller]


def test_default_response_cap_rejects_oversized_mutation_without_retry():
    calls = []

    def handle(request):
        calls.append(request.url.path)
        return httpx.Response(201, content=b" " * (4 * 1024 * 1024 + 1),
                              headers={"Content-Type": "application/json"})

    with CoolerpClient(ClientOptions(transport=httpx.MockTransport(handle))) as client:
        result = client.journal_entries.create(
            actor="sdk-cap-test", entry_date="2025-01-15", journal_code="GEN",
            idempotency_key=str(uuid4()),
            lines=[dict(account_code="1000", debit=10000, credit=0),
                   dict(account_code="4000", debit=0, credit=10000)],
        )
        assert isinstance(result, Err)
        assert result.error["code"] == "RESPONSE_TOO_LARGE"
        assert result.error["retryable"] == "never"
    assert calls == ["/journal-entries"]
