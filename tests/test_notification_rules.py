import sys, os; sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), 'runtime'))
import unittest
from forma_runtime.notification_rules import NotificationRules


class FakeClock:
    def __init__(self, start=1000):
        self.t = start

    def __call__(self):
        self.t += 1
        return self.t


class NotificationRulesTests(unittest.TestCase):
    def setUp(self):
        self.rules = NotificationRules(FakeClock())

    def test_add_rule_stores_fields(self):
        r = self.rules.add_rule("message", scope="conv-1", threshold=3,
                                window_seconds=600, now=55)
        self.assertEqual(r["kind"], "message")
        self.assertEqual(r["scope"], "conv-1")
        self.assertEqual(r["threshold"], 3)
        self.assertEqual(r["window_seconds"], 600)
        self.assertTrue(r["enabled"])
        self.assertEqual(r["created_at"], 55)
        self.assertFalse(r["removed"])

    def test_rule_ids_stable_and_never_reused(self):
        a = self.rules.add_rule("message")
        b = self.rules.add_rule("message")
        self.assertNotEqual(a["id"], b["id"])
        self.assertTrue(a["id"].startswith("nrule-"))

    def test_empty_kind_rejected(self):
        with self.assertRaises(ValueError):
            self.rules.add_rule("   ")

    def test_non_string_scope_rejected(self):
        with self.assertRaises(ValueError):
            self.rules.add_rule("message", scope=7)

    def test_zero_threshold_rejected(self):
        with self.assertRaises(ValueError):
            self.rules.add_rule("message", threshold=0)

    def test_bool_threshold_rejected(self):
        with self.assertRaises(ValueError):
            self.rules.add_rule("message", threshold=True)

    def test_negative_window_rejected(self):
        with self.assertRaises(ValueError):
            self.rules.add_rule("message", window_seconds=-5)

    def test_zero_window_rejected(self):
        with self.assertRaises(ValueError):
            self.rules.add_rule("message", window_seconds=0)

    def test_missing_rule_get_raises(self):
        with self.assertRaises(KeyError):
            self.rules.get("nrule-999999")

    def test_removed_rule_get_raises(self):
        r = self.rules.add_rule("message")
        self.rules.remove(r["id"])
        with self.assertRaises(KeyError):
            self.rules.get(r["id"])

    def test_disable_then_enable_flip(self):
        r = self.rules.add_rule("message")
        self.assertTrue(self.rules.disable(r["id"]))
        self.assertFalse(self.rules.get(r["id"])["enabled"])
        self.assertTrue(self.rules.enable(r["id"]))
        self.assertTrue(self.rules.get(r["id"])["enabled"])

    def test_double_disable_returns_false(self):
        r = self.rules.add_rule("message")
        self.assertTrue(self.rules.disable(r["id"]))
        self.assertFalse(self.rules.disable(r["id"]))

    def test_double_enable_returns_false(self):
        r = self.rules.add_rule("message")
        self.assertFalse(self.rules.enable(r["id"]))

    def test_disable_removed_rule_raises(self):
        r = self.rules.add_rule("message")
        self.rules.remove(r["id"])
        with self.assertRaises(KeyError):
            self.rules.disable(r["id"])

    def test_remove_returns_false_when_already_removed(self):
        r = self.rules.add_rule("message")
        self.assertTrue(self.rules.remove(r["id"]))
        self.assertFalse(self.rules.remove(r["id"]))

    def test_list_rules_newest_first(self):
        self.rules.add_rule("message", now=10)
        self.rules.add_rule("pin", now=20)
        self.rules.add_rule("export", now=30)
        listed = self.rules.list_rules()
        self.assertEqual([r["kind"] for r in listed], ["export", "pin", "message"])

    def test_list_rules_filters_by_scope(self):
        self.rules.add_rule("message", scope="a")
        self.rules.add_rule("pin", scope="b")
        listed = self.rules.list_rules(scope="a")
        self.assertEqual(len(listed), 1)
        self.assertEqual(listed[0]["scope"], "a")

    def test_list_rules_hides_removed_by_default(self):
        r = self.rules.add_rule("message")
        self.rules.remove(r["id"])
        self.assertEqual(self.rules.list_rules(), [])
        self.assertEqual(len(self.rules.list_rules(include_removed=True)), 1)

    def test_evaluate_fires_at_threshold(self):
        self.rules.add_rule("message", threshold=3)
        events = [{"type": "message", "ts": 1}, {"type": "message", "ts": 2},
                  {"type": "message", "ts": 3}]
        fired = self.rules.evaluate(events, now=100)
        self.assertEqual(len(fired), 1)
        self.assertEqual(fired[0]["count"], 3)
        self.assertEqual(fired[0]["kind"], "message")

    def test_evaluate_below_threshold_is_silent(self):
        self.rules.add_rule("message", threshold=2)
        fired = self.rules.evaluate([{"type": "message", "ts": 1}], now=100)
        self.assertEqual(fired, [])

    def test_evaluate_other_kind_ignored(self):
        self.rules.add_rule("message", threshold=1)
        fired = self.rules.evaluate([{"type": "pin", "ts": 1}], now=100)
        self.assertEqual(fired, [])

    def test_empty_scope_matches_any_event_scope(self):
        self.rules.add_rule("message", scope="", threshold=2)
        events = [{"type": "message", "scope": "conv-a", "ts": 1},
                  {"type": "message", "scope": "conv-b", "ts": 2}]
        fired = self.rules.evaluate(events, now=100)
        self.assertEqual(len(fired), 1)
        self.assertEqual(fired[0]["count"], 2)

    def test_scoped_rule_requires_exact_scope(self):
        self.rules.add_rule("message", scope="conv-a", threshold=2)
        events = [{"type": "message", "scope": "conv-a", "ts": 1},
                  {"type": "message", "scope": "conv-b", "ts": 2}]
        fired = self.rules.evaluate(events, now=100)
        self.assertEqual(fired, [])

    def test_window_lower_boundary_exclusive(self):
        self.rules.add_rule("message", threshold=2, window_seconds=600)
        events = [{"type": "message", "ts": 400},
                  {"type": "message", "ts": 500}]
        fired = self.rules.evaluate(events, now=1000)
        self.assertEqual(fired, [])

    def test_window_upper_boundary_inclusive(self):
        self.rules.add_rule("message", threshold=2, window_seconds=600)
        events = [{"type": "message", "ts": 500},
                  {"type": "message", "ts": 1000}]
        fired = self.rules.evaluate(events, now=1000)
        self.assertEqual(len(fired), 1)
        self.assertEqual(fired[0]["count"], 2)

    def test_no_window_means_all_history(self):
        self.rules.add_rule("message", threshold=2)
        events = [{"type": "message", "ts": 5}, {"type": "message", "ts": 900}]
        fired = self.rules.evaluate(events, now=1000)
        self.assertEqual(len(fired), 1)

    def test_disabled_rule_never_fires(self):
        r = self.rules.add_rule("message", threshold=1)
        self.rules.disable(r["id"])
        fired = self.rules.evaluate([{"type": "message", "ts": 1}], now=100)
        self.assertEqual(fired, [])

    def test_removed_rule_never_fires(self):
        r = self.rules.add_rule("message", threshold=1)
        self.rules.remove(r["id"])
        fired = self.rules.evaluate([{"type": "message", "ts": 1}], now=100)
        self.assertEqual(fired, [])

    def test_evaluate_stateless_repeat_calls_identical(self):
        self.rules.add_rule("message", threshold=2)
        events = [{"type": "message", "ts": 1}, {"type": "message", "ts": 2}]
        first = self.rules.evaluate(events, now=100)
        second = self.rules.evaluate(events, now=100)
        self.assertEqual(first, second)

    def test_fired_results_sorted_by_rule_id(self):
        a = self.rules.add_rule("message", threshold=1)
        b = self.rules.add_rule("pin", threshold=1)
        fired = self.rules.evaluate(
            [{"type": "pin", "ts": 1}, {"type": "message", "ts": 2}], now=100)
        self.assertEqual([f["rule_id"] for f in fired], [a["id"], b["id"]])

    def test_events_missing_ts_ignored_inside_window(self):
        self.rules.add_rule("message", threshold=2, window_seconds=50)
        fired = self.rules.evaluate(
            [{"type": "message"}, {"type": "message"}], now=100)
        self.assertEqual(fired, [])

    def test_non_dict_events_ignored(self):
        self.rules.add_rule("message", threshold=2)
        fired = self.rules.evaluate(["message", 5, None,
                                     {"type": "message", "ts": 1},
                                     {"type": "message", "ts": 2}], now=100)
        self.assertEqual(len(fired), 1)

    def test_non_number_now_rejected(self):
        with self.assertRaises(ValueError):
            self.rules.evaluate([], now="soon")

    def test_evaluate_output_copies_not_references(self):
        r = self.rules.add_rule("message", threshold=1)
        fired = self.rules.evaluate([{"type": "message", "ts": 1}], now=100)
        fired[0]["kind"] = "mutated"
        self.assertEqual(self.rules.get(r["id"])["kind"], "message")


if __name__ == "__main__":
    unittest.main()
