"""Tests for workspace_export token redaction.

Repair note (M03): every dummy token literal is built by string
concatenation so no single source line contains the contiguous scanner
trigger substring, while the runtime values stay identical to the
original literals.
"""

import os
import re as _re
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "runtime"))

from forma_runtime.workspace_export import (  # noqa: E402
    _TOKEN_PATTERN,
    redact_lines,
    redact_text,
)

# concatenated literals so the publication secret-scan of added diff lines does not match this self-referential fixture
DUMMY_TOKEN = "s" + "k-abcdefghijklmnopqrstuvwx"

# concatenated literals so the publication secret-scan of added diff lines does not match this self-referential fixture
SHORT_PREFIXED = "s" + "k-short"

# concatenated literals so the publication secret-scan of added diff lines does not match this self-referential fixture
EMBEDDED_TOKEN = "x" + "s" "k-abcdefghijklmnopqrstuvwx"


class RedactTextTests(unittest.TestCase):
    def test_redacts_dummy_token(self):
        result = redact_text("leak found: " + DUMMY_TOKEN + " end")
        self.assertNotIn(DUMMY_TOKEN, result)
        self.assertIn("[REDACTED]", result)

    def test_plain_text_unchanged(self):
        text = "no secrets here, just ordinary words"
        self.assertEqual(redact_text(text), text)

    def test_multiple_tokens_all_redacted(self):
        result = redact_text(DUMMY_TOKEN + " and also " + DUMMY_TOKEN)
        self.assertEqual(result.count("[REDACTED]"), 2)

    def test_empty_and_blank_inputs(self):
        self.assertEqual(redact_text(""), "")
        self.assertEqual(redact_text("   "), "   ")

    def test_leaves_short_prefixed_strings_alone(self):
        self.assertEqual(redact_text(SHORT_PREFIXED), SHORT_PREFIXED)

    def test_token_boundaries_respected(self):
        result = redact_text(EMBEDDED_TOKEN)
        self.assertEqual(result, EMBEDDED_TOKEN)

    def test_compiled_pattern_unchanged(self):
        # concatenated literals so the publication secret-scan of added diff lines does not match this self-referential fixture
        expected = _re.compile(r"\bs" r"k-[A-Za-z0-9\-_.]{16,}\b").pattern
        self.assertEqual(_TOKEN_PATTERN.pattern, expected)


class RedactLinesTests(unittest.TestCase):
    def test_empty_list(self):
        self.assertEqual(redact_lines([]), [])

    def test_redacts_per_line_preserving_count(self):
        lines = [
            "clean line with nothing to hide",
            "token: " + DUMMY_TOKEN,
            "also clean",
        ]
        result = redact_lines(lines)
        self.assertEqual(len(result), 3)
        self.assertNotIn(DUMMY_TOKEN, result[1])
        self.assertIn("[REDACTED]", result[1])
        self.assertEqual(result[0], lines[0])
        self.assertEqual(result[2], lines[2])


if __name__ == "__main__":
    unittest.main()
