import os
import sys
import unittest
from dataclasses import dataclass
from datetime import datetime, timedelta, timezone

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..")))

from runtime.forma_runtime.composio_actions import (
    ActionGrant,
    ActionNotGranted,
    ActionReceipt,
    ActionRequest,
    ComposioTransport,
    GrantBook,
    GrantExpired,
    GrantRevoked,
    StubTransport,
    UnknownConnectorGrant,
    redact,
)


def _request(action="send_email", connector="gmail", request_id="req-1"):
    return ActionRequest(
        request_id=request_id,
        connector=connector,
        action=action,
        arguments={"to": "a@example.com"},
        justification="operator asked to send the summary",
    )


def _book(now, actions=("send_email",), revoked=False, expires=None):
    book = GrantBook()
    book.grant(
        ActionGrant(
            connector="gmail",
            allowed_actions=set(actions),
            granted_at=now - timedelta(hours=1),
            expires_at=expires if expires is not None else now + timedelta(hours=1),
            revoked=revoked,
        )
    )
    return book


class ValidationTests(unittest.TestCase):
    def setUp(self):
        self.now = datetime(2026, 1, 1, 12, 0, 0, tzinfo=timezone.utc)

    def test_unknown_connector(self):
        book = _book(self.now)
        with self.assertRaises(UnknownConnectorGrant):
            book.validate(
                ActionRequest(
                    "r", "slack", "send_message", {}, "why"
                ),
                self.now,
            )

    def test_revoked(self):
        book = _book(self.now, revoked=True)
        with self.assertRaises(GrantRevoked):
            book.validate(_request(), self.now)

    def test_expired_boundary(self):
        expires = self.now
        book = _book(self.now, expires=expires)
        with self.assertRaises(GrantExpired):
            book.validate(_request(), self.now)

    def test_expired_after(self):
        book = _book(self.now, expires=self.now - timedelta(seconds=1))
        with self.assertRaises(GrantExpired):
            book.validate(_request(), self.now)

    def test_not_expired_before_boundary(self):
        book = _book(self.now, expires=self.now + timedelta(seconds=1))
        grant = book.validate(_request(), self.now)
        self.assertEqual(grant.connector, "gmail")

    def test_action_not_granted(self):
        book = _book(self.now, actions=("list_emails",))
        with self.assertRaises(ActionNotGranted):
            book.validate(_request(action="send_email"), self.now)

    def test_revoke_helper(self):
        book = _book(self.now)
        book.revoke("gmail")
        with self.assertRaises(GrantRevoked):
            book.validate(_request(), self.now)


class RedactionTests(unittest.TestCase):
    def test_masks_sensitive_keys_case_insensitive(self):
        payload = {
            "api_key": "abc",
            "AccessToken": "x",
            "my_secret": "y",
            "UserPassword": "z",
            "Keyboard": "masked-too",  # fail-closed substring match
            "plain": "kept",
        }
        out = redact(payload)
        self.assertEqual(
            out["api_key"], "***REDACTED***"
        )
        self.assertEqual(out["AccessToken"], "***REDACTED***")
        self.assertEqual(out["my_secret"], "***REDACTED***")
        self.assertEqual(out["UserPassword"], "***REDACTED***")
        self.assertEqual(out["Keyboard"], "***REDACTED***")
        self.assertEqual(out["plain"], "kept")

    def test_nested_dicts(self):
        payload = {
            "result": {"auth": {"refresh_token": "t"}, "count": 3},
            "items": [{"api_key": "k"}],
        }
        out = redact(payload)
        self.assertEqual(
            out["result"]["auth"]["refresh_token"], "***REDACTED***"
        )
        self.assertEqual(out["result"]["count"], 3)

    def test_non_dict_passthrough(self):
        self.assertEqual(redact("s"), "s")
        self.assertEqual(redact(5), 5)

    def test_receipt_status_validation(self):
        for status in ("succeeded", "failed", "denied"):
            ActionReceipt(request_id="r", status=status)
        with self.assertRaises(ValueError):
            ActionReceipt(request_id="r", status="pending")


class TransportTests(unittest.TestCase):
    def test_abstract(self):
        with self.assertRaises(TypeError):
            ComposioTransport()  # type: ignore[abstract]

    def test_stub_records_and_returns_canned(self):
        canned = [
            ActionReceipt(request_id="req-1", status="succeeded", sanitized_payload={"id": "1"}),
            ActionReceipt(request_id="req-2", status="failed", sanitized_payload={"error": "boom"}),
        ]
        stub = StubTransport(receipts=canned)
        r1 = stub.execute(_request(request_id="req-1"))
        r2 = stub.execute(_request(request_id="req-2", action="list_emails"))
        self.assertEqual(
            [r.request_id for r in (r1, r2)], ["req-1", "req-2"]
        )
        self.assertEqual(r1.status, "succeeded")
        self.assertEqual(r1.sanitized_payload, {"id": "1"})
        self.assertEqual(r2.status, "failed")
        self.assertEqual(
            [q.request_id for q in stub.recorded_requests], ["req-1", "req-2"]
        )
        self.assertEqual(
            [q.action for q in stub.recorded_requests],
            ["send_email", "list_emails"],
        )

    def test_stub_default_receipt(self):
        stub = StubTransport()
        receipt = stub.execute(_request(request_id="req-9"))
        self.assertEqual(receipt.request_id, "req-9")
        self.assertEqual(receipt.status, "succeeded")
        self.assertEqual(len(stub.recorded_requests), 1)


if __name__ == "__main__":
    unittest.main()
