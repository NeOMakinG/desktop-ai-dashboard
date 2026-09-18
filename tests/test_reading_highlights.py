"""Tests for reading-list highlights (F25)."""

import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "runtime"))

import unittest

from forma_runtime.reading_highlights import (
    MAX_CONTEXT_CHARS,
    MAX_NOTE_CHARS,
    MAX_TEXT_CHARS,
    HighlightStore,
    PALETTE,
)


class FakeClock:
    def __init__(self, start=1000):
        self.value = start

    def tick(self, seconds=1):
        self.value += seconds
        return self.value

    def __call__(self):
        return self.value


def make_store():
    clock = FakeClock()
    return HighlightStore(clock), clock


class HighlightStoreTest(unittest.TestCase):
    def test_add_returns_complete_record(self):
        store, _ = make_store()
        item = store.add_highlight("e1", "salient line", now=10)
        self.assertTrue(item["id"].startswith("hl-"))
        self.assertEqual(item["entry_id"], "e1")
        self.assertEqual(item["text"], "salient line")
        self.assertEqual(item["color"], "yellow")
        self.assertEqual(item["note"], "")
        self.assertIsNone(item["anchor"])
        self.assertFalse(item["resolved"])
        self.assertFalse(item["removed"])
        self.assertEqual(item["created_at"], 10)
        self.assertEqual(item["updated_at"], 10)

    def test_ids_are_sequential(self):
        store, clock = make_store()
        one = store.add_highlight("e1", "a", now=clock.tick())
        two = store.add_highlight("e1", "b", now=clock.tick())
        self.assertNotEqual(one["id"], two["id"])

    def test_add_requires_nonempty_text(self):
        store, _ = make_store()
        for bad in ("", "   ", None, 5):
            with self.assertRaises(ValueError):
                store.add_highlight("e1", bad)

    def test_add_rejects_oversize_text(self):
        store, _ = make_store()
        with self.assertRaises(ValueError):
            store.add_highlight("e1", "x" * (MAX_TEXT_CHARS + 1))

    def test_add_rejects_unknown_color(self):
        store, _ = make_store()
        with self.assertRaises(ValueError):
            store.add_highlight("e1", "t", color="beige")

    def test_add_rejects_non_string_note(self):
        store, _ = make_store()
        with self.assertRaises(ValueError):
            store.add_highlight("e1", "t", note=7)

    def test_add_rejects_oversize_note(self):
        store, _ = make_store()
        with self.assertRaises(ValueError):
            store.add_highlight("e1", "t", note="n" * (MAX_NOTE_CHARS + 1))

    def test_update_text_note_color(self):
        store, _ = make_store()
        item = store.add_highlight("e1", "t", now=10)
        out = store.update_highlight(item["id"], text="t2", note="why",
                                     color="green")
        self.assertEqual(out["text"], "t2")
        self.assertEqual(out["note"], "why")
        self.assertEqual(out["color"], "green")
        self.assertGreater(out["updated_at"], out["created_at"])
        self.assertEqual(out["created_at"], 10)

    def test_update_unknown_id_raises(self):
        store, _ = make_store()
        with self.assertRaises(KeyError):
            store.update_highlight("hl-9999", note="x")

    def test_set_resolved_round_trip(self):
        store, _ = make_store()
        item = store.add_highlight("e1", "t", now=10)
        out = store.set_resolved(item["id"])
        self.assertTrue(out["resolved"])
        back = store.set_resolved(item["id"], resolved=False)
        self.assertFalse(back["resolved"])

    def test_delete_soft_removes(self):
        store, _ = make_store()
        item = store.add_highlight("e1", "t")
        store.delete(item["id"])
        self.assertEqual(store.list_highlights(), [])
        self.assertEqual(store.list_highlights(include_removed=True)[0]["id"],
                         item["id"])

    def test_operations_on_removed_id_raise(self):
        store, _ = make_store()
        item = store.add_highlight("e1", "t")
        store.delete(item["id"])
        with self.assertRaises(KeyError):
            store.update_highlight(item["id"], note="x")
        with self.assertRaises(KeyError):
            store.set_resolved(item["id"])

    def test_get_unknown_raises(self):
        store, _ = make_store()
        with self.assertRaises(KeyError):
            store.get("hl-404")

    def test_mutating_result_does_not_touch_store(self):
        store, _ = make_store()
        item = store.add_highlight("e1", "t")
        item["text"] = "hacked"
        self.assertEqual(store.get(item["id"])["text"], "t")


class HighlightListingTest(unittest.TestCase):
    def _seed(self):
        store, _ = make_store()
        a = store.add_highlight("e1", "first", color="yellow", now=10)
        b = store.add_highlight("e1", "second", color="green", now=11)
        c = store.add_highlight("e2", "other", color="green", now=12)
        return store, a, b, c

    def test_list_sorted_by_entry_then_time(self):
        store, a, b, c = self._seed()
        rows = store.list_highlights()
        self.assertEqual([r["id"] for r in rows],
                         [a["id"], b["id"], c["id"]])

    def test_list_filters_by_entry(self):
        store, a, b, c = self._seed()
        rows = store.list_highlights(entry_id="e2")
        self.assertEqual([r["id"] for r in rows], [c["id"]])

    def test_list_filters_by_color(self):
        store, a, b, c = self._seed()
        rows = store.list_highlights(color="green")
        self.assertEqual([r["id"] for r in rows], [b["id"], c["id"]])

    def test_list_filters_by_resolved(self):
        store, a, b, c = self._seed()
        store.set_resolved(b["id"])
        self.assertEqual([r["id"] for r in store.list_highlights(resolved=True)],
                         [b["id"]])
        self.assertEqual([r["id"] for r in store.list_highlights(resolved=False)],
                         [a["id"], c["id"]])

    def test_search_matches_text_case_insensitive(self):
        store, a, b, c = self._seed()
        rows = store.search("SECOND")
        self.assertEqual([r["id"] for r in rows], [b["id"]])

    def test_search_matches_note(self):
        store, a, b, c = self._seed()
        store.update_highlight(a["id"], note="follow up with team")
        rows = store.search("follow up")
        self.assertEqual([r["id"] for r in rows], [a["id"]])

    def test_search_blank_returns_empty(self):
        store, a, b, c = self._seed()
        self.assertEqual(store.search(""), [])
        self.assertEqual(store.search(None), [])
        self.assertEqual(store.search("   "), [])

    def test_search_excludes_removed(self):
        store, a, b, c = self._seed()
        store.delete(b["id"])
        self.assertEqual(store.search("second"), [])

    def test_counts_total_resolved_open(self):
        store, a, b, c = self._seed()
        store.set_resolved(a["id"])
        counts = store.counts()
        self.assertEqual(counts, {"total": 3, "resolved": 1, "open": 2})

    def test_counts_scoped_to_entry(self):
        store, a, b, c = self._seed()
        self.assertEqual(store.counts(entry_id="e1"),
                         {"total": 2, "resolved": 0, "open": 2})

    def test_palette_is_exposed_for_renderers(self):
        self.assertIn("yellow", PALETTE)
        self.assertEqual(len(PALETTE), len(set(PALETTE)))


class HighlightAnchorTest(unittest.TestCase):
    def test_anchor_is_normalized_and_stored(self):
        store, _ = make_store()
        item = store.add_highlight(
            "e1", "t", anchor={"offset": 42, "prefix": "before ",
                               "suffix": " after", "extra": "dropped"},
            now=5)
        self.assertEqual(item["anchor"],
                         {"offset": 42, "prefix": "before ", "suffix": " after"})

    def test_anchor_offset_must_be_non_negative_int(self):
        store, _ = make_store()
        for bad in (-1, "0", 1.5, True, None):
            with self.assertRaises(ValueError):
                store.add_highlight("e1", "t", anchor={"offset": bad})

    def test_anchor_context_length_is_capped(self):
        store, _ = make_store()
        with self.assertRaises(ValueError):
            store.add_highlight("e1", "t",
                                anchor={"offset": 0,
                                        "prefix": "p" * (MAX_CONTEXT_CHARS + 1)})

    def test_anchor_must_be_dict_or_none(self):
        store, _ = make_store()
        with self.assertRaises(ValueError):
            store.add_highlight("e1", "t", anchor="offset0")

    def test_unanchored_sorts_before_anchored_in_entry(self):
        store, _ = make_store()
        late = store.add_highlight("e1", "anchored",
                                   anchor={"offset": 7}, now=1)
        early = store.add_highlight("e1", "unanchored", now=2)
        rows = store.list_highlights(entry_id="e1")
        self.assertEqual([r["id"] for r in rows], [early["id"], late["id"]])

    def test_duplicate_at_same_anchor_with_same_text_is_rejected(self):
        store, _ = make_store()
        store.add_highlight("e1", "same", anchor={"offset": 3})
        with self.assertRaises(ValueError):
            store.add_highlight("e1", "same", anchor={"offset": 3})

    def test_same_anchor_allows_different_text(self):
        store, _ = make_store()
        store.add_highlight("e1", "same", anchor={"offset": 3})
        out = store.add_highlight("e1", "different", anchor={"offset": 3})
        self.assertEqual(store.counts(entry_id="e1")["total"], 2)
        self.assertEqual(out["color"], "yellow")

    def test_duplicate_allowed_again_after_delete(self):
        store, _ = make_store()
        first = store.add_highlight("e1", "same", anchor={"offset": 3})
        store.delete(first["id"])
        again = store.add_highlight("e1", "same", anchor={"offset": 3})
        self.assertNotEqual(first["id"], again["id"])

    def test_to_dict_round_trip(self):
        store, _ = make_store()
        store.add_highlight("e1", "t", note="n", color="pink",
                            anchor={"offset": 1})
        blob = store.to_dict()
        self.assertEqual(len(blob["highlights"]), 1)
        blob["highlights"][0]["text"] = "mutated"
        self.assertEqual(store.list_highlights()[0]["text"], "t")


if __name__ == "__main__":
    unittest.main()
