"""Tests for the pure notification snooze store.

Covers runtime/forma_runtime/notification_snooze.py: snooze creation
and validation, (rule_id, minutes) dedupe and re-arm semantics,
cancel/get copy isolation, due() wake boundary and ordering, and
pending() filtering and partitioning against due().
"""

import os
import sys
import unittest

sys.path.insert(
    0,
    os.path.join(
        os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "runtime"
    ),
)

from forma_runtime.notification_snooze import (
    MAX_MINUTES,
    MAX_REASON_CHARS,
    MIN_MINUTES,
    SnoozeStore,
)


class FakeClock:
    def __init__(self, start=1000.0):
        self.now = start

    def __call__(self):
        return self.now

    def advance(self, seconds):
        self.now += seconds


class SnoozeStoreTest(unittest.TestCase):
    def test_snooze_row_shape_and_injected_now(self):
        store = SnoozeStore(FakeClock())
        row, created = store.snooze("nrule-1", 15, reason="focus", now=5000)
        self.assertTrue(created)
        self.assertEqual(row["id"], "snooze-000001")
        self.assertEqual(row["rule_id"], "nrule-1")
        self.assertEqual(row["minutes"], 15)
        self.assertEqual(row["reason"], "focus")
        self.assertEqual(row["created_at"], 5000)
        self.assertEqual(row["wake_at"], 5000 + 15 * 60)
        self.assertFalse(row["removed"])

    def test_clock_used_when_now_omitted(self):
        clock = FakeClock(start=100.0)
        store = SnoozeStore(clock)
        row, created = store.snooze("nrule-1", 5)
        self.assertTrue(created)
        self.assertEqual(row["created_at"], 100.0)
        self.assertEqual(row["wake_at"], 400.0)

    def test_duplicate_live_pair_is_noop(self):
        store = SnoozeStore(FakeClock())
        first, _ = store.snooze("nrule-1", 15, reason="focus", now=1000)
        second, created = store.snooze("nrule-1", 15, reason="other", now=1100)
        self.assertFalse(created)
        self.assertEqual(second["id"], first["id"])
        self.assertEqual(second["reason"], "focus")
        self.assertEqual(second["wake_at"], 1000 + 15 * 60)

    def test_same_rule_different_minutes_are_independent(self):
        store = SnoozeStore(FakeClock())
        a, ca = store.snooze("nrule-1", 5, now=1000)
        b, cb = store.snooze("nrule-1", 30, now=1000)
        self.assertTrue(ca)
        self.assertTrue(cb)
        self.assertNotEqual(a["id"], b["id"])

    def test_rearm_after_expiry_keeps_id(self):
        store = SnoozeStore(FakeClock())
        first, _ = store.snooze("nrule-1", 5, now=1000)
        again, created = store.snooze("nrule-1", 5, reason="again", now=2000)
        self.assertTrue(created)
        self.assertEqual(again["id"], first["id"])
        self.assertEqual(again["wake_at"], 2000 + 5 * 60)
        self.assertEqual(again["reason"], "again")

    def test_rearm_after_cancel_keeps_id(self):
        store = SnoozeStore(FakeClock())
        first, _ = store.snooze("nrule-1", 5, now=1000)
        store.cancel(first["id"])
        again, created = store.snooze("nrule-1", 5, now=1100)
        self.assertTrue(created)
        self.assertEqual(again["id"], first["id"])
        self.assertFalse(again["removed"])
        self.assertEqual(again["wake_at"], 1100 + 5 * 60)

    def test_cancel_is_idempotent_and_missing_raises(self):
        store = SnoozeStore(FakeClock())
        row, _ = store.snooze("nrule-1", 5, now=1000)
        self.assertTrue(store.cancel(row["id"]))
        self.assertFalse(store.cancel(row["id"]))
        with self.assertRaises(KeyError):
            store.cancel("snooze-999999")

    def test_get_live_and_removed(self):
        store = SnoozeStore(FakeClock())
        row, _ = store.snooze("nrule-1", 5, now=1000)
        self.assertEqual(store.get(row["id"])["id"], row["id"])
        store.cancel(row["id"])
        with self.assertRaises(KeyError):
            store.get(row["id"])

    def test_get_returns_copy(self):
        store = SnoozeStore(FakeClock())
        row, _ = store.snooze("nrule-1", 5, reason="focus", now=1000)
        got = store.get(row["id"])
        got["reason"] = "tampered"
        self.assertEqual(store.get(row["id"])["reason"], "focus")

    def test_validation_errors(self):
        store = SnoozeStore(FakeClock())
        for bad_rule in ("", "   ", None, 5):
            with self.assertRaises(ValueError):
                store.snooze(bad_rule, 5, now=1000)
        for bad_minutes in (0, -5, True, False, 5.0, "5", MAX_MINUTES + 1):
            with self.assertRaises(ValueError):
                store.snooze("nrule-1", bad_minutes, now=1000)
        with self.assertRaises(ValueError):
            store.snooze("nrule-1", 5, reason=None, now=1000)
        with self.assertRaises(ValueError):
            store.snooze(
                "nrule-1", 5, reason="x" * (MAX_REASON_CHARS + 1), now=1000
            )

    def test_bounds_are_inclusive(self):
        store = SnoozeStore(FakeClock())
        big, _ = store.snooze("nrule-1", MAX_MINUTES, now=1000)
        self.assertEqual(big["minutes"], MAX_MINUTES)
        small, _ = store.snooze("nrule-2", MIN_MINUTES, now=1000)
        self.assertEqual(small["minutes"], 1)

    def test_clock_must_be_callable(self):
        with self.assertRaises(TypeError):
            SnoozeStore("not callable")


class SnoozeDueTest(unittest.TestCase):
    def test_empty_when_nothing_snoozed(self):
        self.assertEqual(SnoozeStore(FakeClock()).due(now=1000), [])

    def test_boundary_wake_at_is_due(self):
        store = SnoozeStore(FakeClock())
        row, _ = store.snooze("nrule-1", 5, now=1000)
        self.assertEqual(store.due(now=row["wake_at"] - 1), [])
        due = store.due(now=row["wake_at"])
        self.assertEqual([r["id"] for r in due], [row["id"]])

    def test_due_excludes_cancelled(self):
        store = SnoozeStore(FakeClock())
        row, _ = store.snooze("nrule-1", 5, now=1000)
        store.cancel(row["id"])
        self.assertEqual(store.due(now=row["wake_at"] + 99), [])

    def test_due_ordering_wake_then_id(self):
        store = SnoozeStore(FakeClock())
        late, _ = store.snooze("nrule-b", 30, now=1000)
        early, _ = store.snooze("nrule-a", 10, now=1000)
        due = store.due(now=3000)
        self.assertEqual([r["id"] for r in due], [early["id"], late["id"]])

    def test_due_tiebreaks_on_id(self):
        store = SnoozeStore(FakeClock())
        b, _ = store.snooze("nrule-b", 5, now=1000)  # created first
        a, _ = store.snooze("nrule-a", 5, now=1000)
        due = store.due(now=2000)
        self.assertEqual([r["id"] for r in due], [b["id"], a["id"]])

    def test_clock_advance_drives_due(self):
        clock = FakeClock(start=1000.0)
        store = SnoozeStore(clock)
        row, _ = store.snooze("nrule-1", 5)
        self.assertEqual(store.due(), [])
        clock.advance(5 * 60)
        self.assertEqual([r["id"] for r in store.due()], [row["id"]])

    def test_rearmed_pair_appears_once_with_same_id(self):
        store = SnoozeStore(FakeClock())
        first, _ = store.snooze("nrule-1", 5, now=1000)
        store.snooze("nrule-1", 5, now=2000)
        due = store.due(now=2000 + 5 * 60)
        self.assertEqual(len(due), 1)
        self.assertEqual(due[0]["id"], first["id"])


class SnoozePendingTest(unittest.TestCase):
    def test_pending_excludes_due_and_cancelled(self):
        store = SnoozeStore(FakeClock())
        woke, _ = store.snooze("nrule-1", 5, now=1000)
        dead, _ = store.snooze("nrule-2", 60, now=1000)
        asleep, _ = store.snooze("nrule-3", 60, now=1000)
        store.cancel(dead["id"])
        pend = store.pending(now=2000)
        self.assertEqual([r["id"] for r in pend], [asleep["id"]])
        self.assertEqual([r["id"] for r in store.due(now=2000)], [woke["id"]])

    def test_pending_rule_filter(self):
        store = SnoozeStore(FakeClock())
        store.snooze("nrule-1", 5, now=1000)
        b, _ = store.snooze("nrule-2", 50, now=1000)
        pend = store.pending(rule_id="nrule-2", now=1100)
        self.assertEqual([r["id"] for r in pend], [b["id"]])
        self.assertEqual(store.pending(rule_id="nrule-x", now=1100), [])

    def test_pending_and_due_partition_live_rows(self):
        store = SnoozeStore(FakeClock())
        rows = [
            store.snooze("nrule-%d" % i, 5 * i, now=1000)[0]
            for i in range(1, 5)
        ]
        now = 1000 + 5 * 2 * 60
        pend = store.pending(now=now)
        due = store.due(now=now)
        ids = sorted(r["id"] for r in pend + due)
        self.assertEqual(ids, sorted(r["id"] for r in rows))

    def test_pending_ordering(self):
        store = SnoozeStore(FakeClock())
        second, _ = store.snooze("nrule-1", 30, now=1000)
        first, _ = store.snooze("nrule-2", 5, now=1000)
        pend = store.pending(now=1001)
        self.assertEqual([r["id"] for r in pend], [first["id"], second["id"]])

    def test_reason_roundtrip_and_copy_isolation(self):
        store = SnoozeStore(FakeClock())
        row, _ = store.snooze("nrule-1", 5, reason="standup", now=1000)
        pend = store.pending(now=1001)
        pend[0]["reason"] = "tampered"
        self.assertEqual(store.get(row["id"])["reason"], "standup")


if __name__ == "__main__":
    unittest.main()
