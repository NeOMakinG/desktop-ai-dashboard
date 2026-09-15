"""Tests for runtime/forma_runtime/context_envelope.py (unittest layout follows
tests/test_cron_proposals.py conventions)."""

import datetime
import json
import os
import sys
import unittest

sys.path.insert(
    0,
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "runtime"),
)

from forma_runtime.context_envelope import (  # noqa: E402
    PROVENANCE,
    SCHEMA_VERSION,
    build_app_state_envelope,
)

FIXED_NOW = datetime.datetime(2026, 9, 14, 12, 0, 0, tzinfo=datetime.timezone.utc)


class ContextEnvelopeTests(unittest.TestCase):
    def test_header_fields(self):
        env = build_app_state_envelope(now=FIXED_NOW)
        self.assertEqual(env["schema_version"], "1")
        self.assertEqual(SCHEMA_VERSION, "1")
        self.assertEqual(env["generated_at_utc"], "2026-09-14T12:00:00Z")
        self.assertEqual(env["provenance"], "forma-runtime app.state.read envelope")
        self.assertEqual(PROVENANCE, "forma-runtime app.state.read envelope")

    def test_allowlist_drops_unknown_fields(self):
        env = build_app_state_envelope(
            workspaces=[
                {
                    "id": "ws-1",
                    "title": "Inbox triage",
                    "updated_at": "2026-09-12T08:30:00Z",
                    "message_bodies": ["hello"],
                    "chat_history": [{"role": "user", "text": "hi"}],
                    "account_payload": {"email": "a@example.com"},
                }
            ],
            interfaces=[{"id": "if-1", "name": "Board", "revision": 3, "status": "active", "html": "<script>"}],
            grants=[{"id": "gr-1", "scope": "mail:read", "status": "active", "expiry": "2026-12-01T00:00:00Z", "receipt": {"token": "x"}}],
            automation={"total": 1, "items": [{"id": "au-1", "state": "idle", "last_output": "big blob"}]},
            now=FIXED_NOW,
        )
        blob = json.dumps(env)
        for banned in ("message_bodies", "chat_history", "account_payload", "html", "receipt", "last_output", "hello", "<script>"):
            self.assertNotIn(banned, blob)
        self.assertEqual(env["workspaces"][0]["title"], "Inbox triage")
        self.assertEqual(env["grants"][0]["scope"], "mail:read")
        self.assertEqual(env["automation"]["items"][0]["state"], "idle")

    def test_deep_credential_redaction_nested_dicts_and_lists(self):
        env = build_app_state_envelope(
            workspaces=[
                {
                    "id": "ws-9",
                    "title": {"nested": [{"api_key": "k"}, {"safe": "keep"}]},
                    "updated_at": "2026-09-12T08:30:00Z",
                }
            ],
            interfaces=[],
            grants=[
                {
                    "id": "gr-9",
                    "scope": ["mail:read", {"ok": 1, "Api_Token": "t"}],
                    "status": "active",
                    "expiry": "2026-12-01T00:00:00Z",
                }
            ],
            now=FIXED_NOW,
        )
        self.assertEqual(env["redactions"], 2)
        blob = json.dumps(env).replace(" ", "")
        self.assertNotIn("api_key", blob.lower())
        self.assertNotIn("api_token", blob.lower())
        self.assertNotIn('"k"', blob)
        self.assertNotIn('"t"', blob)
        self.assertIn('"safe":"keep"', blob)
        self.assertIn('"ok":1', blob)

    def test_item_cap_truncation_with_retained_counts(self):
        workspaces = [
            {"id": "ws-%02d" % i, "title": "w" + str(i), "updated_at": "2026-09-01T00:00:00Z"}
            for i in range(7)
        ]
        env = build_app_state_envelope(workspaces=workspaces, max_items=3, now=FIXED_NOW)
        self.assertTrue(env["truncated"])
        self.assertEqual(len(env["workspaces"]), 3)
        self.assertEqual(env["retained_counts"]["workspaces"], 3)
        self.assertEqual([w["id"] for w in env["workspaces"]], ["ws-00", "ws-01", "ws-02"])

    def test_byte_cap_truncation_with_retained_counts(self):
        workspaces = [
            {
                "id": "ws-%03d" % i,
                "title": "workspace with a reasonably long title number %d" % i,
                "updated_at": "2026-09-01T00:00:00Z",
            }
            for i in range(60)
        ]
        env = build_app_state_envelope(workspaces=workspaces, max_bytes=8192, now=FIXED_NOW)
        blob = json.dumps(env, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
        self.assertLessEqual(len(blob.encode("utf-8")), 8192)
        self.assertTrue(env["truncated"])
        self.assertLess(env["retained_counts"]["workspaces"], 60)
        self.assertEqual(env["retained_counts"]["workspaces"], len(env["workspaces"]))

    def test_determinism_identical_inputs_identical_output(self):
        workspaces = [
            {"id": "ws-2", "title": "B", "updated_at": "2026-09-02T00:00:00Z"},
            {"id": "ws-1", "title": "A", "updated_at": "2026-09-01T00:00:00Z"},
        ]
        grants = [
            {"id": "gr-2", "scope": "cal:read", "status": "active", "expiry": "2026-11-01T00:00:00Z"},
            {"id": "gr-1", "scope": "mail:read", "status": "revoked", "expiry": "2026-10-01T00:00:00Z"},
        ]
        kwargs = dict(
            workspaces=workspaces,
            interfaces=[{"id": "if-1", "name": "Cal", "revision": 2, "status": "active"}],
            grants=grants,
            automation={"total": 2, "items": [{"id": "au-2", "state": "running"}, {"id": "au-1", "state": "idle"}]},
            now=FIXED_NOW,
        )
        first = json.dumps(build_app_state_envelope(**kwargs), sort_keys=True)
        second = json.dumps(build_app_state_envelope(**kwargs), sort_keys=True)
        self.assertEqual(first, second)
        env = json.loads(first)
        self.assertEqual([w["id"] for w in env["workspaces"]], ["ws-1", "ws-2"])
        self.assertEqual([g["id"] for g in env["grants"]], ["gr-1", "gr-2"])
        self.assertEqual([i["id"] for i in env["automation"]["items"]], ["au-1", "au-2"])

    def test_realistic_fixture_zero_redactions_no_chat_history(self):
        env = build_app_state_envelope(
            workspaces=[
                {"id": "ws-101", "title": "Weekly triage", "updated_at": "2026-09-13T18:42:07Z"},
                {"id": "ws-42", "title": "Renewal watch", "updated_at": "2026-09-11T09:15:00Z"},
            ],
            interfaces=[
                {"id": "if-7", "name": "Inbox board", "revision": 12, "status": "active"},
                {"id": "if-3", "name": "Calendar strip", "revision": 4, "status": "draft"},
            ],
            grants=[
                {"id": "gr-18", "scope": "mail:read", "status": "active", "expiry": "2026-12-01T00:00:00Z"},
                {"id": "gr-19", "scope": "cal:read", "status": "expiring", "expiry": "2026-09-30T00:00:00Z"},
            ],
            automation={
                "total": 3,
                "items": [
                    {"id": "au-5", "state": "running"},
                    {"id": "au-6", "state": "idle"},
                    {"id": "au-7", "state": "paused"},
                ],
            },
            now=FIXED_NOW,
        )
        self.assertEqual(env["redactions"], 0)
        self.assertFalse(env["truncated"])
        blob = json.dumps(env).lower()
        for banned in ("chat_history", "message", "body", "payload", "api_key", "token", "secret", "password", "authorization", "cookie"):
            self.assertNotIn(banned, blob)
        self.assertEqual(env["retained_counts"], {"workspaces": 2, "interfaces": 2, "grants": 2, "automation_items": 3})


if __name__ == "__main__":
    unittest.main()
