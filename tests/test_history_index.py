import sys, os; sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "runtime"))
import unittest

from forma_runtime.history_index import HistoryIndex


class FakeClock:
    def __init__(self, start=1000):
        self.now = start

    def tick(self, step=100):
        self.now += step
        return self.now

    def __call__(self):
        return self.now


class TestRecordVisit(unittest.TestCase):
    def setUp(self):
        self.clock = FakeClock()
        self.index = HistoryIndex(self.clock)

    def test_first_visit_fields_and_defaults(self):
        entry = self.index.record_visit("https://a.example/inbox", "Inbox")
        self.assertEqual(entry["id"], "hist-000001")
        self.assertEqual(entry["url"], "https://a.example/inbox")
        self.assertEqual(entry["title"], "Inbox")
        self.assertEqual(entry["visit_count"], 1)
        self.assertEqual(entry["first_visit_at"], 1000)
        self.assertEqual(entry["last_visit_at"], 1000)

    def test_repeat_visit_increments_and_refreshes(self):
        self.index.record_visit("https://a.example/", "A")
        self.clock.tick()
        again = self.index.record_visit("https://a.example/", "A2")
        self.assertEqual(again["visit_count"], 2)
        self.assertEqual(again["last_visit_at"], 1100)
        self.assertEqual(again["first_visit_at"], 1000)
        self.assertEqual(again["id"], "hist-000001")
        self.assertEqual(self.index.entry_count(), 1)

    def test_repeat_visit_empty_title_keeps_title(self):
        self.index.record_visit("https://a.example/", "A")
        again = self.index.record_visit("https://a.example/")
        self.assertEqual(again["title"], "A")

    def test_returns_fresh_copies_not_live_records(self):
        first = self.index.record_visit("https://a.example/", "A")
        first["visit_count"] = 99
        current = self.index.get_entry("https://a.example/")
        self.assertEqual(current["visit_count"], 1)

    def test_rejects_empty_and_oversized_urls(self):
        with self.assertRaises(ValueError):
            self.index.record_visit("")
        with self.assertRaises(ValueError):
            self.index.record_visit("   ")
        with self.assertRaises(ValueError):
            self.index.record_visit("https://a.example/" + "x" * 2048)

    def test_explicit_now_overrides_clock(self):
        entry = self.index.record_visit("https://a.example/", "A", now=42)
        self.assertEqual(entry["last_visit_at"], 42.0)
        self.assertEqual(entry["first_visit_at"], 42.0)


class TestSearch(unittest.TestCase):
    def setUp(self):
        self.clock = FakeClock()
        self.index = HistoryIndex(self.clock)
        self.index.record_visit("https://mail.example/inbox", "Inbox mail", now=100)
        self.index.record_visit("https://docs.example/guide", "Guide", now=200)
        self.index.record_visit("https://mail.example/sent", "Sent", now=150)
        self.index.record_visit("https://mail.example/sent", "Sent", now=300)

    def test_matches_url_and_title_case_insensitive(self):
        # Two distinct entries match; the repeated sent visit is one entry.
        self.assertEqual(len(self.index.search("MAIL")), 2)
        titles = [e["title"] for e in self.index.search("guide")]
        self.assertEqual(titles, ["Guide"])

    def test_ranking_count_then_recency_then_id(self):
        ranked = self.index.search("mail.example")
        self.assertEqual(ranked[0]["url"], "https://mail.example/sent")
        self.assertEqual(ranked[0]["visit_count"], 2)

    def test_equal_counts_ranked_by_recency_then_id(self):
        index = HistoryIndex(FakeClock())
        index.record_visit("https://b.example/", "B", now=100)
        index.record_visit("https://a.example/", "A", now=100)
        index.record_visit("https://c.example/", "C", now=300)
        ranked = index.search("example")
        self.assertEqual(
            [e["url"] for e in ranked],
            ["https://c.example/", "https://b.example/", "https://a.example/"],
        )

    def test_empty_query_returns_empty_list(self):
        self.assertEqual(self.index.search(""), [])
        self.assertEqual(self.index.search("   "), [])

    def test_limit_bounds_and_invalid_rejected(self):
        self.assertEqual(len(self.index.search("mail.example", limit=2)), 2)
        with self.assertRaises(ValueError):
            self.index.search("mail.example", limit=0)

    def test_no_match_returns_empty(self):
        self.assertEqual(self.index.search("nope"), [])


class TestTopSitesAndRecent(unittest.TestCase):
    def setUp(self):
        self.clock = FakeClock()
        self.index = HistoryIndex(self.clock)

    def test_top_sites_ranked_by_visits(self):
        self.index.record_visit("https://a.example/", "A", now=100)
        self.index.record_visit("https://b.example/", "B", now=200)
        self.index.record_visit("https://b.example/", "B", now=300)
        top = self.index.top_sites()
        self.assertEqual(
            [e["url"] for e in top],
            ["https://b.example/", "https://a.example/"],
        )

    def test_top_sites_limit(self):
        for i in range(5):
            self.index.record_visit("https://%d.example/" % i, "T%d" % i, now=i)
        self.assertEqual(len(self.index.top_sites(limit=3)), 3)
        with self.assertRaises(ValueError):
            self.index.top_sites(limit=0)

    def test_recent_orders_by_last_visit(self):
        self.index.record_visit("https://a.example/", "A", now=100)
        self.index.record_visit("https://b.example/", "B", now=200)
        self.index.record_visit("https://a.example/", "A", now=300)
        rec = self.index.recent()
        self.assertEqual(
            [e["url"] for e in rec],
            ["https://a.example/", "https://b.example/"],
        )

    def test_forget_removes_entry(self):
        self.index.record_visit("https://a.example/", "A", now=100)
        self.assertTrue(self.index.forget("https://a.example/"))
        self.assertFalse(self.index.forget("https://a.example/"))
        self.assertIsNone(self.index.get_entry("https://a.example/"))
        self.assertEqual(self.index.entry_count(), 0)
        self.assertEqual(self.index.top_sites(), [])


if __name__ == "__main__":
    unittest.main()
