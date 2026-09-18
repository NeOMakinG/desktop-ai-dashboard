import sys, os; sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "runtime"))
import unittest

from forma_runtime.workspace_search import WorkspaceSearch


class FakeClock:
    def __init__(self, start=1000):
        self.now = start

    def tick(self, step=100):
        self.now += step
        return self.now

    def __call__(self):
        return self.now


class TestRegister(unittest.TestCase):
    def setUp(self):
        self.clock = FakeClock()
        self.index = WorkspaceSearch(self.clock)

    def test_register_fields_and_stable_id(self):
        doc = self.index.register("chat", "conv-1", "Trip planning", text="pack bags")
        self.assertEqual(doc["id"], "doc-000001")
        self.assertEqual(doc["source"], "chat")
        self.assertEqual(doc["key"], "conv-1")
        self.assertEqual(doc["title"], "Trip planning")
        self.assertEqual(doc["text"], "pack bags")
        self.assertEqual(doc["registered_at"], 1000)

    def test_reregister_same_pair_replaces_and_keeps_id(self):
        first = self.index.register("notes", "n-1", "Old title", text="old")
        second = self.index.register("notes", "n-1", "New title", text="new")
        self.assertEqual(second["id"], first["id"])
        self.assertEqual(self.index.count(), 1)
        hits = self.index.search("New title")
        self.assertEqual(len(hits), 1)
        self.assertEqual(hits[0]["text"], "new")
        self.assertEqual(self.index.search("old"), [])

    def test_different_sources_same_key_are_distinct(self):
        self.index.register("chat", "k", "Chat doc", now=100)
        self.index.register("notes", "k", "Notes doc", now=200)
        self.assertEqual(self.index.count(), 2)

    def test_rejects_empty_source_key_and_title(self):
        with self.assertRaises(ValueError):
            self.index.register("", "k", "T")
        with self.assertRaises(ValueError):
            self.index.register("chat", "", "T")
        with self.assertRaises(ValueError):
            self.index.register("chat", "k", "  ")
        with self.assertRaises(ValueError):
            self.index.register("chat", "k", "T", text=123)

    def test_explicit_now_overrides_clock(self):
        doc = self.index.register("chat", "k", "T", now=7)
        self.assertEqual(doc["registered_at"], 7.0)


class TestSearchRanking(unittest.TestCase):
    def setUp(self):
        self.clock = FakeClock()
        self.index = WorkspaceSearch(self.clock)
        self.index.register("chat", "c1", "Flight to Oslo", text="boarding pass", url="https://x/f", now=100)
        self.index.register("notes", "n1", "Oslo packing list", text="warm socks", now=200)
        self.index.register("tabs", "t1", "Weather forecast", text="Oslo in June", now=300)
        self.index.register("dash", "d1", "Trip dashboard", text="Oslo itinerary summary", now=400)

    def test_title_match_outranks_text_only_match(self):
        hits = self.index.search("oslo")
        self.assertEqual(
            [d["source"] for d in hits],
            ["chat", "notes", "dash", "tabs"],
        )

    def test_source_ties_ranked_alphabetically(self):
        hits = self.index.search("summary")
        self.assertEqual([d["source"] for d in hits], ["dash"])

    def test_text_and_url_are_searchable(self):
        self.assertEqual([d["id"] for d in self.index.search("boarding")], ["doc-000001"])
        self.assertEqual([d["id"] for d in self.index.search("https://x")], ["doc-000001"])

    def test_case_insensitive_substring(self):
        hits = self.index.search("OSLO pack")
        self.assertEqual([d["source"] for d in hits], ["notes"])

    def test_empty_query_returns_empty_list(self):
        self.assertEqual(self.index.search(""), [])
        self.assertEqual(self.index.search("  "), [])

    def test_limit_bounds_and_invalid_rejected(self):
        self.assertEqual(len(self.index.search("oslo", limit=2)), 2)
        with self.assertRaises(ValueError):
            self.index.search("oslo", limit=0)

    def test_no_match_returns_empty_list(self):
        self.assertEqual(self.index.search("kyoto"), [])


class TestSourceManagement(unittest.TestCase):
    def setUp(self):
        self.clock = FakeClock()
        self.index = WorkspaceSearch(self.clock)

    def test_sources_sorted_distinct(self):
        self.index.register("notes", "n1", "A")
        self.index.register("chat", "c1", "B")
        self.index.register("chat", "c2", "C")
        self.assertEqual(self.index.sources(), ["chat", "notes"])

    def test_clear_source_removes_only_that_source(self):
        self.index.register("notes", "n1", "A", now=100)
        self.index.register("chat", "c1", "B", now=200)
        removed = self.index.clear_source("chat")
        self.assertEqual(removed, 1)
        self.assertEqual(self.index.count(), 1)
        self.assertEqual(self.index.sources(), ["notes"])
        self.assertEqual(self.index.search("B"), [])

    def test_clear_source_invalid_rejected(self):
        with self.assertRaises(ValueError):
            self.index.clear_source("")


if __name__ == "__main__":
    unittest.main()
