import os
import sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import unittest
from datetime import datetime, timezone

from runtime.forma_runtime.digest import build_digest, format_digest_text


class BuildDigestTests(unittest.TestCase):
    def test_empty_event_list(self):
        now = 1789012345
        digest = build_digest([], now)
        self.assertEqual(digest["date"], datetime.fromtimestamp(now, tz=timezone.utc).date().isoformat())
        self.assertEqual(digest["generated_at"], now)
        self.assertEqual(digest["groups"], [])

    def test_grouping_counts(self):
        now = 1789012345
        events = [
            {"type": "message", "ts": now - 600, "payload": "status ping from Dana"},
            {"type": "message", "ts": now - 400, "payload": "reply from Ines"},
            {"type": "pin", "ts": now - 300, "payload": "pinned the roadmap card"},
            {"type": "export", "ts": now - 200, "payload": "exported weekly totals"},
            {"type": "message", "ts": now - 100, "payload": "wrap up note"},
        ]
        digest = build_digest(events, now)
        types = [g["type"] for g in digest["groups"]]
        self.assertEqual(types, ["message", "export", "pin"])
        counts = {g["type"]: g["count"] for g in digest["groups"]}
        self.assertEqual(counts, {"message": 3, "export": 1, "pin": 1})
        # Empty known types omitted.
        self.assertNotIn("tab_open", counts)
        self.assertNotIn("command", counts)

    def test_24h_cutoff_boundary(self):
        now = 1789012345
        boundary = now - 86400
        events = [
            {"type": "message", "ts": boundary, "payload": "exactly at boundary, excluded"},
            {"type": "message", "ts": boundary + 1, "payload": "one second inside window"},
            {"type": "message", "ts": now, "payload": "exactly now, included"},
            {"type": "message", "ts": now + 5, "payload": "in the future, excluded"},
        ]
        digest = build_digest(events, now)
        self.assertEqual(len(digest["groups"]), 1)
        group = digest["groups"][0]
        self.assertEqual(group["count"], 2)
        payloads = [item["payload"] for item in group["items"]]
        self.assertEqual(payloads, ["one second inside window", "exactly now, included"])

    def test_deterministic_ordering_with_equal_ts(self):
        now = 1789012345
        ts = now - 3600
        events = [
            {"type": "command", "ts": ts, "payload": "first inserted"},
            {"type": "command", "ts": ts, "payload": "second inserted"},
            {"type": "command", "ts": ts - 60, "payload": "earlier ts"},
            {"type": "command", "ts": ts, "payload": "third inserted"},
        ]
        digest = build_digest(events, now)
        payloads = [item["payload"] for item in digest["groups"][0]["items"]]
        self.assertEqual(payloads, ["earlier ts", "first inserted", "second inserted", "third inserted"])

    def test_unknown_type_lands_in_other(self):
        now = 1789012345
        events = [
            {"type": "message", "ts": now - 100, "payload": "plain message"},
            {"type": "browser_refresh", "ts": now - 90, "payload": "unrecognized event"},
            {"type": "voice_note", "ts": now - 80, "payload": "another unrecognized"},
        ]
        digest = build_digest(events, now)
        counts = {g["type"]: g["count"] for g in digest["groups"]}
        self.assertEqual(counts, {"message": 1, "other": 2})

    def test_group_order_count_desc_then_type_asc(self):
        now = 1789012345
        events = [
            {"type": "pin", "ts": now - 10, "payload": "pin one"},
            {"type": "pin", "ts": now - 20, "payload": "pin two"},
            {"type": "export", "ts": now - 30, "payload": "export one"},
            {"type": "export", "ts": now - 40, "payload": "export two"},
            {"type": "message", "ts": now - 50, "payload": "message one"},
        ]
        digest = build_digest(events, now)
        types = [g["type"] for g in digest["groups"]]
        self.assertEqual(types, ["export", "pin", "message"])


class FormatDigestTextTests(unittest.TestCase):
    def test_contains_group_headers_counts_and_lines(self):
        now = 1789012345
        events = [
            {"type": "message", "ts": now - 3661, "payload": "morning check-in from Priya"},
            {"type": "pin", "ts": now - 120, "payload": "pinned delivery checklist"},
        ]
        digest = build_digest(events, now)
        text = format_digest_text(digest)

        self.assertIn("### message (1)", text)
        self.assertIn("### pin (1)", text)

        expected_message_time = datetime.fromtimestamp(now - 3661, tz=timezone.utc).strftime("%H:%M")
        expected_pin_time = datetime.fromtimestamp(now - 120, tz=timezone.utc).strftime("%H:%M")
        self.assertIn(" - {} morning check-in from Priya".format(expected_message_time), text)
        self.assertIn(" - {} pinned delivery checklist".format(expected_pin_time), text)

        lines = text.split("\n")
        self.assertEqual(lines.count("### message (1)"), 1)
        self.assertTrue(any(line.startswith(" - ") for line in lines))

    def test_empty_digest_renders_empty_string(self):
        now = 1789012345
        digest = build_digest([], now)
        self.assertEqual(format_digest_text(digest), "")


if __name__ == "__main__":
    unittest.main()
