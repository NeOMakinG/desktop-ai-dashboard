import os
import sys
sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import unittest

from forma_runtime.tab_sessions import TabSessionStore, group_tabs_by_domain


class FakeClock:
    def __init__(self, timestamps):
        self._timestamps = list(timestamps)
        self._i = 0

    def __call__(self):
        value = self._timestamps[min(self._i, len(self._timestamps) - 1)]
        self._i += 1
        return value


RESEARCH_TABS = [
    {"url": "https://example.com/papers", "title": "Papers"},
    {"url": "http://news.example.org:8080/feed", "title": "Feed"},
]


class TestSaveListRoundtrip(unittest.TestCase):
    def test_save_returns_session_dict_with_counter_ids(self):
        store = TabSessionStore(FakeClock([100]))
        s1 = store.save_session("Research", RESEARCH_TABS)
        s2 = store.save_session("Reading", [])
        self.assertEqual(s1["id"], "sess-0001")
        self.assertEqual(s2["id"], "sess-0002")
        self.assertEqual(s1["name"], "Research")
        self.assertEqual(s1["saved_at"], 100)

    def test_get_session_roundtrips_identical_tab_dicts(self):
        store = TabSessionStore(FakeClock([100]))
        saved = store.save_session("Research", RESEARCH_TABS)
        got = store.get_session(saved["id"])
        self.assertEqual(got["tabs"], RESEARCH_TABS)

    def test_list_sessions_empty_when_none_saved(self):
        store = TabSessionStore(FakeClock([100]))
        self.assertEqual(store.list_sessions(), [])

    def test_saved_snapshot_is_isolated_from_caller_mutation(self):
        store = TabSessionStore(FakeClock([100]))
        tabs = [{"url": "https://a.example", "title": "A"}]
        saved = store.save_session("S", tabs)
        tabs[0]["title"] = "Mutated"
        self.assertEqual(saved["tabs"][0]["title"], "A")


class TestListOrdering(unittest.TestCase):
    def test_sorted_by_saved_at_desc(self):
        store = TabSessionStore(FakeClock([100, 300, 200]))
        store.save_session("a", [])
        store.save_session("b", [])
        store.save_session("c", [])
        self.assertEqual([s["id"] for s in store.list_sessions()],
                         ["sess-0002", "sess-0003", "sess-0001"])

    def test_equal_timestamps_break_tie_by_id_asc(self):
        store = TabSessionStore(FakeClock([100, 100, 100]))
        store.save_session("a", [])
        store.save_session("b", [])
        store.save_session("c", [])
        self.assertEqual([s["id"] for s in store.list_sessions()],
                         ["sess-0001", "sess-0002", "sess-0003"])


class TestGroupSessions(unittest.TestCase):
    def test_duplicate_names_group_together_sorted_asc(self):
        store = TabSessionStore(FakeClock([200, 100, 400, 50]))
        store.save_session("Research", [])  # sess-0001, t=200
        store.save_session("News", [])     # sess-0002, t=100
        store.save_session("Research", [])  # sess-0003, t=400
        store.save_session("Research", [])  # sess-0004, t=50
        groups = store.group_sessions()
        research = groups["Research"]
        # saved_at asc: sess-0004 (50), sess-0001 (200), sess-0003 (400)
        self.assertEqual([s["id"] for s in research],
                         ["sess-0004", "sess-0001", "sess-0003"])
        self.assertEqual([s["id"] for s in groups["News"]], ["sess-0002"])


class TestGroupTabsByDomain(unittest.TestCase):
    def test_http_https_ports_and_userinfo(self):
        counts = group_tabs_by_domain([
            {"url": "https://Example.COM/path", "title": "a"},
            {"url": "http://example.com:8080/other", "title": "b"},
            {"url": "https://user:pass@secret.example.net/x", "title": "c"},
            {"url": "https://secret.example.net/y", "title": "d"},
        ])
        self.assertEqual(counts, {
            "example.com": 2,
            "secret.example.net": 2,
        })

    def test_invalid_or_missing_urls_go_to_other(self):
        counts = group_tabs_by_domain([
            {"url": "not a url at all", "title": "x"},
            {"title": "no url key"},
            {"url": "", "title": "empty"},
        ])
        self.assertEqual(counts, {"other": 3})

    def test_empty_input(self):
        self.assertEqual(group_tabs_by_domain([]), {})


class TestGetDeleteSemantics(unittest.TestCase):
    def test_get_missing_returns_none(self):
        store = TabSessionStore(FakeClock([100]))
        self.assertIsNone(store.get_session("sess-9999"))

    def test_delete_true_once_then_false(self):
        store = TabSessionStore(FakeClock([100]))
        saved = store.save_session("Research", RESEARCH_TABS)
        self.assertTrue(store.delete_session(saved["id"]))
        self.assertFalse(store.delete_session(saved["id"]))
        self.assertIsNone(store.get_session(saved["id"]))

    def test_delete_missing_returns_false(self):
        store = TabSessionStore(FakeClock([100]))
        self.assertFalse(store.delete_session("sess-9999"))


class TestDeterminism(unittest.TestCase):
    def _build(self):
        store = TabSessionStore(FakeClock([300, 100, 200]))
        store.save_session("Research", RESEARCH_TABS)
        store.save_session("Research", [])
        store.save_session("News", RESEARCH_TABS)
        return store

    def test_two_stores_same_inputs_produce_identical_output(self):
        a, b = self._build(), self._build()
        self.assertEqual(a.list_sessions(), b.list_sessions())
        self.assertEqual(a.group_sessions(), b.group_sessions())

    def test_ordering_stable_across_repeated_calls(self):
        store = self._build()
        self.assertEqual(store.list_sessions(), store.list_sessions())
        self.assertEqual(store.group_sessions(), store.group_sessions())


if __name__ == "__main__":
    unittest.main()
