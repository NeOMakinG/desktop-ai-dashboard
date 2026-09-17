"""Deterministic unittest coverage for the quick notes data plane.

Injected clocks only: no sleeps, no randomness in assertions.
"""

import os
import sys
import unittest
from datetime import datetime, timedelta

_REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(_REPO_ROOT, "runtime"))
sys.path.insert(0, _REPO_ROOT)

from forma_runtime.quick_notes import (  # noqa: E402
    QuickNote,
    delete_note,
    list_notes,
    pin_note,
    save_note,
    unpin_note,
)


class QuickNoteTests(unittest.TestCase):
    def setUp(self):
        self.store = {}
        self.t0 = datetime(2026, 9, 17, 9, 0, 0)

    def test_save_rejects_empty_and_whitespace_text(self):
        with self.assertRaises(ValueError):
            save_note(self.store, "", self.t0)
        with self.assertRaises(ValueError):
            save_note(self.store, "   \n\t ", self.t0)
        self.assertEqual(list_notes(self.store), [])

    def test_save_rejects_text_over_limit(self):
        with self.assertRaises(ValueError):
            save_note(self.store, "word " * 101, self.t0)
        save_note(self.store, "a" * 500, self.t0)  # exactly at the limit is fine
        self.assertEqual(len(list_notes(self.store)), 1)

    def test_save_starts_unpinned_with_injected_now(self):
        note = save_note(self.store, "buy milk before noon", self.t0)
        self.assertIsInstance(note, QuickNote)
        self.assertFalse(note.pinned)
        self.assertEqual(note.created_at, self.t0)
        self.assertEqual(note.updated_at, self.t0)
        self.assertEqual(list_notes(self.store)[0].id, note.id)

    def test_save_generates_unique_ids_at_same_timestamp(self):
        first = save_note(self.store, "call the dentist", self.t0)
        second = save_note(self.store, "water the plants", self.t0)
        third = save_note(self.store, "renew the library loan", self.t0)
        ids = {first.id, second.id, third.id}
        self.assertEqual(len(ids), 3)
        self.assertEqual(len(self.store), 3)

    def test_pin_and_unpin_bump_updated_at(self):
        note = save_note(self.store, "forward the lease letter", self.t0)
        later = self.t0 + timedelta(hours=2)
        pinned = pin_note(self.store, note.id, later)
        self.assertTrue(pinned.pinned)
        self.assertEqual(pinned.updated_at, later)
        self.assertEqual(pinned.created_at, self.t0)
        even_later = later + timedelta(minutes=30)
        unpinned = unpin_note(self.store, note.id, even_later)
        self.assertFalse(unpinned.pinned)
        self.assertEqual(unpinned.updated_at, even_later)

    def test_pin_unknown_id_raises_key_error(self):
        with self.assertRaises(KeyError):
            pin_note(self.store, "missing", self.t0)

    def test_unpin_unknown_id_raises_key_error(self):
        with self.assertRaises(KeyError):
            unpin_note(self.store, "missing", self.t0)

    def test_pinned_first_then_updated_desc(self):
        oldest = save_note(self.store, "oldest entry", self.t0)
        newest = save_note(self.store, "newest entry", self.t0 + timedelta(days=2))
        middle = save_note(
            self.store, "middle entry", self.t0 + timedelta(days=1)
        )
        pin_note(self.store, oldest.id, self.t0 + timedelta(days=3))
        order = [note.id for note in list_notes(self.store)]
        self.assertEqual(order, [oldest.id, newest.id, middle.id])

    def test_equal_timestamps_resolve_by_id_ascending(self):
        first = save_note(self.store, "alpha reminder", self.t0)
        second = save_note(self.store, "beta reminder", self.t0)
        third = save_note(self.store, "gamma reminder", self.t0)
        same_now = self.t0 + timedelta(hours=5)
        for note in (first, second, third):
            pin_note(self.store, note.id, same_now)
        expected = sorted([first.id, second.id, third.id])
        order = [note.id for note in list_notes(self.store)]
        self.assertEqual(order, expected)

    def test_delete_removes_note(self):
        note = save_note(self.store, "temporary thought", self.t0)
        delete_note(self.store, note.id)
        self.assertEqual(list_notes(self.store), [])

    def test_delete_unknown_id_raises_key_error(self):
        with self.assertRaises(KeyError):
            delete_note(self.store, "missing")

    def test_json_round_trip_preserves_fields_and_order(self):
        plain = save_note(self.store, "plain note one", self.t0)
        pinned_note = save_note(
            self.store, "pinned note two", self.t0 + timedelta(minutes=10)
        )
        pin_note(
            self.store, pinned_note.id, self.t0 + timedelta(minutes=20)
        )
        original_order = [note.id for note in list_notes(self.store)]

        restored_store = {}
        for key, record in self.store.items():
            round_tripped = QuickNote.from_json(QuickNote.from_json(record).to_json()).to_json()
            restored_store[key] = round_tripped

        restored_notes = {note.id: note for note in list_notes(restored_store)}
        for note in list_notes(self.store):
            twin = restored_notes[note.id]
            self.assertEqual(twin.text, note.text)
            self.assertEqual(twin.created_at, note.created_at)
            self.assertEqual(twin.updated_at, note.updated_at)
            self.assertEqual(twin.pinned, note.pinned)
        self.assertEqual(plain.pinned, restored_notes[plain.id].pinned)
        restored_order = [note.id for note in list_notes(restored_store)]
        self.assertEqual(restored_order, original_order)
        self.assertEqual(restored_order[0], pinned_note.id)


if __name__ == "__main__":
    unittest.main()
