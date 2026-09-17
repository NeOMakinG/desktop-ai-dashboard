import os
import sys
import unittest

sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "..")))

from runtime.forma_runtime.workspace_export import (  # noqa: E402
    ExportPathError,
    ExportRequest,
    ExportResult,
    ExportWorkspaceError,
    compose_export,
    redact,
    render_markdown,
)

import datetime as dt  # noqa: E402

EXPORTED_AT = dt.datetime(2026, 9, 17, 12, 0, 0, tzinfo=dt.timezone.utc)
HOME = os.path.abspath("/home/tester")


def sample_messages():
    return [
        {"speaker": "user", "timestamp": "2026-09-17T11:00:00Z", "text": "Hello"},
        {
            "speaker": "agent",
            "timestamp": "2026-09-17T11:00:05Z",
            "text": "Hi! How can I help?",
        },
        {"speaker": "system", "timestamp": "2026-09-17T11:00:06Z", "text": "Saved"},
    ]


def sample_notes():
    return [{"text": "Prefers morning summaries"}]


def sample_refs():
    return [{"name": "Weekly board", "id": "ifc-123", "revision": 4}]


class ExportRequestValidationTests(unittest.TestCase):
    def test_rejects_empty_workspace_id(self):
        with self.assertRaises(ExportWorkspaceError):
            ExportRequest.create("", "Name", home_dir=HOME)

    def test_rejects_whitespace_workspace_id(self):
        with self.assertRaises(ExportWorkspaceError):
            ExportRequest.create("   ", "Name", home_dir=HOME)

    def test_accepts_valid_request_without_path(self):
        req = ExportRequest.create("ws-1", "My workspace", home_dir=HOME)
        self.assertEqual(req.workspace_id, "ws-1")
        self.assertIsNone(req.path)

    def test_rejects_relative_path(self):
        with self.assertRaises(ExportPathError):
            ExportRequest.create("ws-1", "Name", "out.md", home_dir=HOME)

    def test_rejects_non_md_extension(self):
        with self.assertRaises(ExportPathError):
            ExportRequest.create(
                "ws-1", "Name", os.path.join(HOME, "export.txt"), home_dir=HOME
            )

    def test_rejects_path_outside_home(self):
        with self.assertRaises(ExportPathError):
            ExportRequest.create(
                "ws-1", "Name", "/etc/export.md", home_dir=HOME
            )

    def test_rejects_parent_traversal(self):
        with self.assertRaises(ExportPathError):
            ExportRequest.create(
                "ws-1",
                "Name",
                os.path.join(HOME, "docs", "..", "..", "escape.md"),
                home_dir=HOME,
            )

    def test_accepts_path_under_home(self):
        req = ExportRequest.create(
            "ws-1", "Name", os.path.join(HOME, "Documents", "w.md"), home_dir=HOME
        )
        self.assertEqual(req.path, os.path.join(HOME, "Documents", "w.md"))


class RenderTests(unittest.TestCase):
    def test_h1_title_and_export_line(self):
        md = render_markdown(sample_messages(), sample_notes(), sample_refs(), EXPORTED_AT, "Budget desk")
        self.assertIn("# Budget desk", md)
        self.assertIn("Exported: 2026-09-17T12:00:00Z", md)

    def test_messages_in_order_with_labels_and_timestamps(self):
        md = render_markdown(sample_messages(), [], [], EXPORTED_AT)
        i_user = md.index("**user** · 2026-09-17T11:00:00Z")
        i_agent = md.index("**agent** · 2026-09-17T11:00:05Z")
        i_system = md.index("**system** · 2026-09-17T11:00:06Z")
        self.assertLess(i_user, i_agent)
        self.assertLess(i_agent, i_system)

    def test_deterministic_output(self):
        a = render_markdown(sample_messages(), sample_notes(), sample_refs(), EXPORTED_AT)
        b = render_markdown(sample_messages(), sample_notes(), sample_refs(), EXPORTED_AT)
        self.assertEqual(a, b)

    def test_code_fence_verbatim(self):
        code = "print('hi')\n`inline`\n```nested```"
        messages = [
            {"speaker": "agent", "timestamp": "t", "text": code, "kind": "code"}
        ]
        md = render_markdown(messages, [], [], EXPORTED_AT)
        self.assertIn("print('hi')", md)
        self.assertIn("```nested```", md)
        # outer fence must be longer than the inner run
        self.assertIn("````\nprint('hi')", md)

    def test_remembered_context_section(self):
        md = render_markdown([], sample_notes(), [], EXPORTED_AT)
        self.assertIn("## Remembered context", md)
        self.assertIn("- Prefers morning summaries", md)

    def test_interface_link_with_revision_pin(self):
        md = render_markdown([], [], sample_refs(), EXPORTED_AT)
        self.assertIn("[Weekly board](interface:ifc-123)", md)
        self.assertIn("revision: 4", md)


class RedactionTests(unittest.TestCase):
    def test_bearer_token_redacted(self):
        self.assertNotIn("Bearer abc123", redact("Bearer abc123def456ghi789"))
        self.assertIn("[REDACTED]", redact("Bearer abc123def456ghi789"))

    def test_api_key_redacted(self):
        self.assertIn("[REDACTED]", redact("api_key: sk-abcdef0123456789abcd"))

    def test_secret_never_reaches_output_in_messages(self):
        messages = [
            {
                "speaker": "user",
                "timestamp": "t",
                "text": "my key is sk-abcdef0123456789abcd ok",
            }
        ]
        md = render_markdown(messages, [], [], EXPORTED_AT)
        self.assertNotIn("sk-abcdef0123456789abcd", md)
        self.assertIn("[REDACTED]", md)

    def test_secret_redacted_inside_code_fence(self):
        messages = [
            {
                "speaker": "agent",
                "timestamp": "t",
                "text": "Authorization: Bearer zzz999yyy888xxx777",
                "kind": "code",
            }
        ]
        md = render_markdown(messages, [], [], EXPORTED_AT)
        self.assertNotIn("zzz999yyy888xxx777", md)
        self.assertIn("[REDACTED]", md)

    def test_secret_redacted_in_notes(self):
        notes = [{"text": "token=aaaa1111bbbb2222cccc3333"}]
        md = render_markdown([], notes, [], EXPORTED_AT)
        self.assertNotIn("aaaa1111bbbb2222cccc3333", md)
        self.assertIn("[REDACTED]", md)


class ComposeResultTests(unittest.TestCase):
    def test_result_counts_and_byte_length(self):
        result = compose_export(
            "ws-1",
            "Budget desk",
            sample_messages(),
            sample_notes(),
            sample_refs(),
            EXPORTED_AT,
            path=os.path.join(HOME, "out.md"),
            home_dir=HOME,
        )
        self.assertIsInstance(result, ExportResult)
        self.assertEqual(result.message_count, 3)
        self.assertEqual(result.note_count, 1)
        self.assertEqual(result.interface_ref_count, 1)
        self.assertEqual(result.byte_length, len(result.markdown.encode("utf-8")))
        self.assertIn("# Budget desk", result.markdown)

    def test_compose_validates_workspace(self):
        with self.assertRaises(ExportWorkspaceError):
            compose_export("", "n", [], [], [], EXPORTED_AT)

    def test_compose_validates_path(self):
        with self.assertRaises(ExportPathError):
            compose_export("ws", "n", [], [], [], EXPORTED_AT, path="/tmp/x.md", home_dir=HOME)


if __name__ == "__main__":
    unittest.main()
