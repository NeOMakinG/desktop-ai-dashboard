"""Tests for runtime/forma_runtime/slash_commands.py (F09 pure slice).

Uses the same repo-root sys.path shim style as tests/test_workspace_export.py.
"""

import os
import sys
import unittest

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..")))

from runtime.forma_runtime.slash_commands import (  # noqa: E402
    COMMANDS,
    ParseError,
    parse,
    route,
    suggest,
)


class RegistryTests(unittest.TestCase):
    def test_every_entry_has_description_and_valid_action_mapping(self):
        seen_names = set()
        for entry in COMMANDS:
            self.assertTrue(entry.name, "entry must have a name")
            self.assertFalse(entry.name.startswith("/"), "name must not include slash")
            self.assertTrue(
                entry.description and entry.description.strip(),
                "entry {!r} must have a non-empty description".format(entry.name),
            )
            self.assertTrue(entry.action, "entry must map to an action")
            self.assertTrue(entry.target, "entry must map to a target")
            self.assertNotIn(entry.name, seen_names, "names must be unique")
            seen_names.add(entry.name)
        self.assertEqual(
            seen_names,
            {"summarize", "export", "pin", "dashboard", "clear-context", "help"},
        )


class ParseTests(unittest.TestCase):
    def test_parse_happy_path_without_args(self):
        parsed = parse("/pin")
        self.assertEqual(parsed.name, "pin")
        self.assertEqual(parsed.raw, "/pin")
        self.assertEqual(parsed.args, {})

    def test_parse_happy_path_with_args(self):
        parsed = parse("/summarize the last ten messages")
        self.assertEqual(parsed.name, "summarize")
        self.assertEqual(parsed.raw, "/summarize the last ten messages")
        self.assertEqual(parsed.args, {"text": "the last ten messages"})

    def test_parse_rejects_non_slash_input(self):
        with self.assertRaises(ParseError) as ctx:
            parse("hello there")
        self.assertEqual(ctx.exception.kind, "not_a_command")
        self.assertEqual(ctx.exception.suggestions, [])

    def test_parse_misspelled_export_suggests_export(self):
        with self.assertRaises(ParseError) as ctx:
            parse("/expor")
        self.assertEqual(ctx.exception.kind, "unknown_command")
        self.assertIn("export", ctx.exception.suggestions)

    def test_parse_unknown_command_lists_near_matches(self):
        with self.assertRaises(ParseError) as ctx:
            parse("/da")
        self.assertEqual(ctx.exception.kind, "unknown_command")
        self.assertIn("dashboard", ctx.exception.suggestions)
        self.assertTrue(ctx.exception.suggestions)

    def test_parse_unknown_with_no_prefix_match_still_errors(self):
        with self.assertRaises(ParseError) as ctx:
            parse("/zzzz")
        self.assertEqual(ctx.exception.kind, "unknown_command")
        self.assertEqual(ctx.exception.suggestions, [])


class SuggestTests(unittest.TestCase):
    def test_suggest_empty_prefix_returns_all_sorted(self):
        entries = suggest("")
        names = [entry.name for entry in entries]
        self.assertEqual(names, [entry.name for entry in COMMANDS if True])
        self.assertEqual(names, sorted(names))
        self.assertEqual(len(names), len(COMMANDS))

    def test_suggest_prefix_filters(self):
        names = [entry.name for entry in suggest("ex")]
        self.assertEqual(names, ["export"])
        names_c = [entry.name for entry in suggest("c")]
        self.assertEqual(names_c, ["clear-context"])

    def test_suggest_no_match_returns_empty(self):
        self.assertEqual(suggest("qq"), [])

    def test_suggest_is_deterministic(self):
        first = [(e.name, e.description) for e in suggest("")]
        second = [(e.name, e.description) for e in suggest("")]
        self.assertEqual(first, second)
        self.assertEqual(
            [e.name for e in suggest("h")],
            [e.name for e in suggest("h")],
        )


class RouteTests(unittest.TestCase):
    def test_route_descriptors_match_action_names(self):
        expected = {
            "summarize": "summarize_thread",
            "export": "workspace_export",
            "pin": "workspace_pin",
            "dashboard": "open_dashboard",
            "clear-context": "clear_context",
            "help": "show_help",
        }
        for entry in COMMANDS:
            parsed = parse("/" + entry.name)
            descriptor = route(parsed)
            self.assertEqual(descriptor["action"], expected[entry.name])
            self.assertEqual(descriptor["target"], entry.target)

    def test_route_output_is_plain_json(self):
        import json

        for entry in COMMANDS:
            descriptor = route(parse("/" + entry.name))
            self.assertEqual(
                json.loads(json.dumps(descriptor)),
                descriptor,
                "descriptor must round-trip through JSON",
            )
            self.assertEqual(set(descriptor.keys()), {"action", "target", "args"})

    def test_route_carries_args(self):
        descriptor = route(parse("/export include the pinned notes"))
        self.assertEqual(descriptor["action"], "workspace_export")
        self.assertEqual(descriptor["args"], {"text": "include the pinned notes"})
        descriptor_no_args = route(parse("/export"))
        self.assertEqual(descriptor_no_args["args"], {"text": ""})

    def test_route_unknown_name_raises(self):
        from runtime.forma_runtime.slash_commands import ParsedCommand

        with self.assertRaises(ValueError):
            route(ParsedCommand(name="nope", args={}, raw="/nope"))


class ImportPurityTests(unittest.TestCase):
    def test_module_imports_only_allowed_stdlib(self):
        import runtime.forma_runtime.slash_commands as mod

        loaded = {
            name
            for name in sys.modules
            if name.startswith("runtime.forma_runtime.slash_commands")
        }
        self.assertTrue(loaded)
        source = open(mod.__file__, "r", encoding="utf-8").read()
        for forbidden in (
            "import os",
            "import sys",
            "import time",
            "import socket",
            "import subprocess",
            "import pathlib",
            "import requests",
            "open(",
        ):
            self.assertNotIn(
                forbidden,
                source,
                "module source must not contain {!r}".format(forbidden),
            )

    def test_module_exposes_no_io_attributes(self):
        import runtime.forma_runtime.slash_commands as mod

        for attr in ("open", "read", "write", "system", "popen", "environ"):
            self.assertFalse(
                hasattr(mod, attr),
                "module must not expose IO attribute {!r}".format(attr),
            )


if __name__ == "__main__":
    unittest.main()
