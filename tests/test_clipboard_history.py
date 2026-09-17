import sys, os; sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "runtime"))
import unittest
from forma_runtime.clipboard_history import ClipboardHistory


class FakeClock:
    def __init__(self, start=1000.0, step=10.0):
        self.now = start
        self.step = step

    def __call__(self):
        value = self.now
        self.now += self.step
        return value


class TestClipboardHistory(unittest.TestCase):
    def setUp(self):
        self.clock = FakeClock()

    def test_copy_stores_fields_and_defaults(self):
        history = ClipboardHistory(self.clock)
        entry = history.copy("hello world")
        self.assertEqual(entry["id"], "clip-000001")
        self.assertEqual(entry["text"], "hello world")
        self.assertEqual(entry["created_at"], 1000.0)
        self.assertFalse(entry["pinned"])

    def test_duplicate_of_newest_refreshes_instead_of_duplicating(self):
        history = ClipboardHistory(self.clock)
        first = history.copy("alpha")
        second = history.copy("beta")
        refreshed = history.copy("beta", now=5000.0)
        self.assertEqual(refreshed["id"], second["id"])
        self.assertEqual(refreshed["created_at"], 5000.0)
        self.assertEqual(len(history.list_entries()), 2)
        self.assertEqual(history.list_entries()[0]["text"], "beta")
        self.assertNotEqual(first["id"], refreshed["id"])

    def test_empty_text_raises_value_error(self):
        history = ClipboardHistory(self.clock)
        with self.assertRaises(ValueError):
            history.copy("")

    def test_oversize_text_raises_value_error(self):
        history = ClipboardHistory(self.clock)
        with self.assertRaises(ValueError):
            history.copy("x" * 100001)
        history.copy("x" * 100000)

    def test_newest_first_order_with_same_clock_resolved_by_id(self):
        history = ClipboardHistory(FakeClock(step=0.0))
        a = history.copy("a")
        b = history.copy("b")
        c = history.copy("c")
        order = [e["id"] for e in history.list_entries()]
        self.assertEqual(order, [c["id"], b["id"], a["id"]])

    def test_pin_moves_to_front_and_unpin_restores(self):
        history = ClipboardHistory(self.clock)
        a = history.copy("a")
        b = history.copy("b")
        c = history.copy("c")
        self.assertTrue(history.pin(a["id"])["pinned"])
        order = [e["id"] for e in history.list_entries()]
        self.assertEqual(order, [a["id"], c["id"], b["id"]])
        history.unpin(a["id"])
        order = [e["id"] for e in history.list_entries()]
        self.assertEqual(order, [c["id"], b["id"], a["id"]])

    def test_limit_and_unpinned_only_filtering(self):
        history = ClipboardHistory(self.clock)
        a = history.copy("a")
        history.copy("b")
        history.copy("c")
        history.pin(a["id"])
        self.assertEqual(len(history.list_entries(limit=2)), 2)
        unpinned = history.list_entries(include_unpinned_only=True)
        self.assertEqual(
            [e["text"] for e in unpinned], ["c", "b"]
        )

    def test_delete_removes_entry(self):
        history = ClipboardHistory(self.clock)
        a = history.copy("a")
        history.copy("b")
        history.delete(a["id"])
        self.assertEqual(
            [e["text"] for e in history.list_entries()], ["b"]
        )

    def test_search_is_case_insensitive_newest_first(self):
        history = ClipboardHistory(self.clock)
        history.copy("Red Apple")
        history.copy("green apple")
        history.copy("banana")
        matches = history.search("APPLE")
        self.assertEqual(
            [e["text"] for e in matches], ["green apple", "Red Apple"]
        )

    def test_to_dict_shape_and_content(self):
        history = ClipboardHistory(self.clock)
        history.copy("a")
        history.copy("b")
        snapshot = history.to_dict()
        self.assertEqual(list(snapshot.keys()), ["entries"])
        self.assertEqual(
            [e["text"] for e in snapshot["entries"]], ["b", "a"]
        )

    def test_unknown_id_raises_key_error_everywhere(self):
        history = ClipboardHistory(self.clock)
        history.copy("a")
        with self.assertRaises(KeyError):
            history.pin("clip-999999")
        with self.assertRaises(KeyError):
            history.unpin("clip-999999")
        with self.assertRaises(KeyError):
            history.delete("clip-999999")

    def test_non_callable_clock_raises_type_error(self):
        with self.assertRaises(TypeError):
            ClipboardHistory(clock=12345)


if __name__ == "__main__":
    unittest.main()
