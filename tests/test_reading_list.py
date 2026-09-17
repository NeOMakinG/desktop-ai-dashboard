import sys, os; sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import unittest

from forma_runtime.reading_list import ReadingListStore


class FakeClock:
    def __init__(self, start=1000):
        self.now = start

    def tick(self, step=100):
        self.now += step
        return self.now

    def __call__(self):
        return self.now


class TestReadingListStore(unittest.TestCase):
    def setUp(self):
        self.clock = FakeClock()
        self.store = ReadingListStore(self.clock)

    def ids(self, entries):
        return [e["id"] for e in entries]

    def test_add_entry_fields_and_defaults(self):
        entry = self.store.add_entry("https://example.org/a", "Article A")
        self.assertEqual(entry["url"], "https://example.org/a")
        self.assertEqual(entry["title"], "Article A")
        self.assertEqual(entry["note"], "")
        self.assertEqual(entry["created_at"], self.clock.now)
        self.assertFalse(entry["read"])
        self.assertFalse(entry["archived"])
        self.assertTrue(entry["id"])

    def test_add_entry_with_note_and_explicit_now(self):
        entry = self.store.add_entry(
            "https://example.org/b", "Article B", note="read for work", now=4242
        )
        self.assertEqual(entry["note"], "read for work")
        self.assertEqual(entry["created_at"], 4242)

    def test_clock_not_callable_raises(self):
        with self.assertRaises(TypeError):
            ReadingListStore(clock=12345)

    def test_unique_ids_at_same_clock_value(self):
        a = self.store.add_entry("https://a.example", "A", now=500)
        b = self.store.add_entry("https://b.example", "B", now=500)
        self.assertNotEqual(a["id"], b["id"])

    def test_list_ordering_unread_first_then_newest_then_id(self):
        self.store.add_entry("https://a.example", "A", now=100)  # oldest unread
        self.store.add_entry("https://b.example", "B", now=300)  # newest unread
        self.store.add_entry("https://c.example", "C", now=200)  # middle unread
        self.store.mark_read("rl-000001")
        order = self.ids(self.store.list_entries())
        self.assertEqual(order, ["rl-000002", "rl-000003", "rl-000001"])

    def test_equal_created_at_ties_order_by_id_ascending(self):
        self.store.add_entry("https://a.example", "A", now=700)
        self.store.add_entry("https://b.example", "B", now=700)
        self.store.add_entry("https://c.example", "C", now=700)
        self.assertEqual(
            self.ids(self.store.list_entries()),
            ["rl-000001", "rl-000002", "rl-000003"],
        )

    def test_mark_read_moves_below_unread(self):
        newest = self.store.add_entry("https://a.example", "A", now=200)
        oldest = self.store.add_entry("https://b.example", "B", now=100)
        # Both unread: newest created_at first.
        self.assertEqual(self.ids(self.store.list_entries()), [newest["id"], oldest["id"]])
        self.store.mark_read(newest["id"])
        # Read entry sinks below the unread one.
        self.assertEqual(self.ids(self.store.list_entries()), [oldest["id"], newest["id"]])

    def test_mark_unread_restores_position(self):
        first = self.store.add_entry("https://a.example", "A", now=300)
        second = self.store.add_entry("https://b.example", "B", now=100)
        self.store.mark_read(first["id"])
        self.assertEqual(self.ids(self.store.list_entries()), [second["id"], first["id"]])
        self.store.mark_unread(first["id"])
        self.assertEqual(
            self.ids(self.store.list_entries()),
            [first["id"], second["id"]],
        )

    def test_archive_hides_from_default_and_shows_with_flag(self):
        entry = self.store.add_entry("https://a.example", "A", now=100)
        other = self.store.add_entry("https://b.example", "B", now=200)
        self.store.archive(entry["id"])
        self.assertEqual(self.ids(self.store.list_entries()), [other["id"]])
        archived_view = self.store.list_entries(include_archived=True)
        self.assertEqual(
            self.ids(archived_view),
            [other["id"], entry["id"]],
        )
        self.assertTrue(archived_view[1]["archived"])

    def test_unarchive_restores_visibility(self):
        entry = self.store.add_entry("https://a.example", "A", now=100)
        self.store.archive(entry["id"])
        self.store.unarchive(entry["id"])
        self.assertEqual(self.ids(self.store.list_entries()), [entry["id"]])

    def test_delete_removes_entry(self):
        entry = self.store.add_entry("https://a.example", "A", now=100)
        self.store.delete(entry["id"])
        self.assertEqual(self.store.list_entries(include_archived=True), [])
        with self.assertRaises(KeyError):
            self.store.delete(entry["id"])

    def test_unknown_id_raises_keyerror_everywhere(self):
        self.store.add_entry("https://a.example", "A", now=100)
        for method in (
            self.store.mark_read,
            self.store.mark_unread,
            self.store.archive,
            self.store.unarchive,
            self.store.delete,
        ):
            with self.assertRaises(KeyError):
                method("rl-999999")

    def test_to_dict_shape_and_content(self):
        entry = self.store.add_entry("https://a.example", "A", note="later", now=250)
        snapshot = self.store.to_dict()
        self.assertEqual(set(snapshot.keys()), {"entries"})
        self.assertEqual(len(snapshot["entries"]), 1)
        stored = snapshot["entries"][0]
        self.assertEqual(stored["id"], entry["id"])
        self.assertEqual(stored["url"], "https://a.example")
        self.assertEqual(stored["title"], "A")
        self.assertEqual(stored["note"], "later")
        self.assertEqual(stored["created_at"], 250)
        self.assertFalse(stored["read"])
        self.assertFalse(stored["archived"])
        # Snapshot includes archived entries for archive views.
        self.store.archive(entry["id"])
        self.assertEqual(len(self.store.to_dict()["entries"]), 1)

    def test_empty_store_lists_and_snapshots(self):
        self.assertEqual(self.store.list_entries(), [])
        self.assertEqual(self.store.to_dict(), {"entries": []})


if __name__ == "__main__":
    unittest.main()
