"""Tests for runtime.forma_runtime.workspace_templates (F10 spike).

Pure unittest coverage over an in-memory backing store; no filesystem,
environment, clock, or network access.
"""

import sys
import unittest
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from runtime.forma_runtime.workspace_templates import (  # noqa: E402
    TemplateNotFoundError,
    TemplateStore,
    TemplateValidationError,
    deserialize_layout,
    serialize_layout,
)


class MemoryStore:
    """In-memory BackingStore: read() -> str, write(text)."""

    def __init__(self, text: str = "") -> None:
        self.text = text
        self.writes: list = []

    def read(self) -> str:
        return self.text

    def write(self, text: str) -> None:
        self.writes.append(text)
        self.text = text


def make_layout(dashboards=None, pins=None):
    return {
        "dashboards": list(dashboards or []),
        "pins": list(pins or []),
    }


class TestSaveListApplyDeleteRoundTrip(unittest.TestCase):
    def test_round_trip(self):
        store = TemplateStore(MemoryStore())
        layout = make_layout(
            dashboards=[{"id": "d1", "title": "Inbox"}],
            pins=[{"id": "p1", "target": "d1"}],
        )
        store.save_template("daily", layout, "2026-09-17T00:00:00Z")

        listed = store.list_templates()
        self.assertEqual(len(listed), 1)
        self.assertEqual(listed[0].name, "daily")
        self.assertEqual(listed[0].created_at, "2026-09-17T00:00:00Z")
        self.assertEqual(listed[0].layout, layout)

        applied = store.apply_template("daily")
        self.assertEqual(applied, layout)

        store.delete_template("daily")
        self.assertEqual(store.list_templates(), [])
        with self.assertRaises(TemplateNotFoundError):
            store.apply_template("daily")


class TestDuplicateOverwrite(unittest.TestCase):
    def test_duplicate_name_overwrites(self):
        mem = MemoryStore()
        store = TemplateStore(mem)
        first = make_layout(dashboards=[{"id": "d1"}])
        second = make_layout(dashboards=[{"id": "d2"}], pins=[{"id": "p1"}])

        store.save_template("daily", first, "2026-09-16T00:00:00Z")
        store.save_template("daily", second, "2026-09-17T00:00:00Z")

        listed = store.list_templates()
        self.assertEqual(len(listed), 1)
        self.assertEqual(listed[0].created_at, "2026-09-17T00:00:00Z")
        self.assertEqual(listed[0].layout, second)
        self.assertEqual(store.apply_template("daily"), second)


class TestNameValidation(unittest.TestCase):
    def test_empty_name_rejected(self):
        store = TemplateStore(MemoryStore())
        with self.assertRaises(TemplateValidationError):
            store.save_template("", make_layout(), "t")

    def test_name_too_long_rejected(self):
        store = TemplateStore(MemoryStore())
        with self.assertRaises(TemplateValidationError):
            store.save_template("a" * 65, make_layout(), "t")

    def test_name_uppercase_rejected(self):
        store = TemplateStore(MemoryStore())
        with self.assertRaises(TemplateValidationError):
            store.save_template("Daily", make_layout(), "t")

    def test_name_underscore_rejected(self):
        store = TemplateStore(MemoryStore())
        with self.assertRaises(TemplateValidationError):
            store.save_template("daily_standup", make_layout(), "t")

    def test_name_max_length_and_slug_chars_accepted(self):
        store = TemplateStore(MemoryStore())
        name = "a" * 64
        store.save_template(name, make_layout(), "t")
        self.assertEqual(store.list_templates()[0].name, name)


class TestLayoutValidation(unittest.TestCase):
    def test_unknown_key_named_in_error(self):
        store = TemplateStore(MemoryStore())
        layout = make_layout()
        layout["columns"] = []
        with self.assertRaises(TemplateValidationError) as ctx:
            store.save_template("daily", layout, "t")
        self.assertIn("columns", str(ctx.exception))

    def test_missing_key_rejected(self):
        store = TemplateStore(MemoryStore())
        with self.assertRaises(TemplateValidationError):
            store.save_template("daily", {"dashboards": []}, "t")

    def test_non_list_rejected(self):
        store = TemplateStore(MemoryStore())
        with self.assertRaises(TemplateValidationError):
            store.save_template("daily", {"dashboards": {}, "pins": []}, "t")


class TestPartialApply(unittest.TestCase):
    def setUp(self):
        self.store = TemplateStore(MemoryStore())
        self.template = make_layout(
            dashboards=[{"id": "td1"}],
            pins=[{"id": "tp1"}],
        )
        self.store.save_template("daily", self.template, "t")
        self.current = make_layout(
            dashboards=[{"id": "cd1"}],
            pins=[{"id": "cp1"}],
        )

    def test_dashboards_only(self):
        applied = self.store.apply_template(
            "daily", self.current, take_dashboards=True, take_pins=False
        )
        self.assertEqual(applied["dashboards"], [{"id": "td1"}])
        self.assertEqual(applied["pins"], [{"id": "cp1"}])

    def test_pins_only(self):
        applied = self.store.apply_template(
            "daily", self.current, take_dashboards=False, take_pins=True
        )
        self.assertEqual(applied["dashboards"], [{"id": "cd1"}])
        self.assertEqual(
            sorted(p["id"] for p in applied["pins"]), ["cp1", "tp1"]
        )

    def test_defaults_take_both(self):
        applied = self.store.apply_template("daily", self.current)
        self.assertEqual(applied["dashboards"], [{"id": "td1"}])
        self.assertEqual(
            sorted(p["id"] for p in applied["pins"]), ["cp1", "tp1"]
        )

    def test_returns_new_object_and_does_not_mutate_inputs(self):
        applied = self.store.apply_template("daily", self.current)
        applied["dashboards"].append({"id": "extra"})
        self.assertEqual(self.current["dashboards"], [{"id": "cd1"}])
        self.assertEqual(self.store.list_templates()[0].layout["dashboards"], [{"id": "td1"}])


class TestPinsUnionById(unittest.TestCase):
    def test_union_and_collision_template_wins(self):
        store = TemplateStore(MemoryStore())
        template = make_layout(
            pins=[
                {"id": "shared", "pos": "template-pos"},
                {"id": "tp1"},
            ]
        )
        store.save_template("daily", template, "t")
        current = make_layout(
            pins=[
                {"id": "shared", "pos": "current-pos"},
                {"id": "cp1"},
            ]
        )
        applied = store.apply_template("daily", current)
        by_id = {p["id"]: p for p in applied["pins"]}
        self.assertEqual(set(by_id), {"shared", "tp1", "cp1"})
        self.assertEqual(by_id["shared"]["pos"], "template-pos")


class TestUnknownTemplate(unittest.TestCase):
    def test_apply_unknown_raises(self):
        store = TemplateStore(MemoryStore())
        with self.assertRaises(TemplateNotFoundError):
            store.apply_template("nope")

    def test_delete_unknown_raises(self):
        store = TemplateStore(MemoryStore())
        with self.assertRaises(TemplateNotFoundError):
            store.delete_template("nope")


class TestDeterministicOrdering(unittest.TestCase):
    def test_list_sorted_by_name_regardless_of_save_order(self):
        store = TemplateStore(MemoryStore())
        for name in ["zeta", "alpha-2", "alpha-1", "mid"]:
            store.save_template(name, make_layout(), "t")
        self.assertEqual(
            [r.name for r in store.list_templates()],
            ["alpha-1", "alpha-2", "mid", "zeta"],
        )

    def test_serialization_round_trip_and_key_order(self):
        layout = make_layout(
            dashboards=[{"id": "d1", "title": "T"}], pins=[{"id": "p1"}],
        )
        text = serialize_layout(layout)
        self.assertEqual(text, serialize_layout(deserialize_layout(text)))
        self.assertIn('"dashboards":', text)
        self.assertIn('"pins":', text)
        self.assertEqual(deserialize_layout(text), layout)

    def test_store_json_is_deterministic(self):
        mem1, mem2 = MemoryStore(), MemoryStore()
        for name in ["b", "a"]:
            for store in (TemplateStore(mem1), TemplateStore(mem2)):
                store.save_template(name, make_layout(dashboards=[{"id": "d"}]), "t")
        self.assertEqual(mem1.text, mem2.text)


if __name__ == "__main__":
    unittest.main()
