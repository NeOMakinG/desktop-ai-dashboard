import sys, os; sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), 'runtime'))
import unittest
from forma_runtime.attachments import AttachmentRegistry


class FakeClock:
    def __init__(self, start=1000):
        self.t = start

    def __call__(self):
        self.t += 1
        return self.t


class AttachmentRegistryTests(unittest.TestCase):
    def setUp(self):
        self.reg = AttachmentRegistry(FakeClock())

    def test_attach_stores_fields(self):
        e = self.reg.attach("conv-1", "/tmp/report.pdf", kind="pdf", now=55)
        self.assertEqual(e["conversation_id"], "conv-1")
        self.assertEqual(e["path"], "/tmp/report.pdf")
        self.assertEqual(e["redacted_path"], "report.pdf")
        self.assertEqual(e["kind"], "pdf")
        self.assertEqual(e["attached_at"], 55)
        self.assertFalse(e["removed"])

    def test_ids_stable_and_never_reused(self):
        a = self.reg.attach("c", "/a.txt")
        b = self.reg.attach("c", "/b.txt")
        self.assertNotEqual(a["id"], b["id"])
        self.assertTrue(a["id"].startswith("att-"))

    def test_empty_conversation_rejected(self):
        with self.assertRaises(ValueError):
            self.reg.attach("   ", "/a.txt")

    def test_blank_path_rejected(self):
        with self.assertRaises(ValueError):
            self.reg.attach("c", "   ")

    def test_oversize_path_rejected(self):
        with self.assertRaises(ValueError):
            self.reg.attach("c", "/" + "x" * 5000)

    def test_non_string_kind_rejected(self):
        with self.assertRaises(ValueError):
            self.reg.attach("c", "/a.txt", kind=7)

    def test_redaction_hides_directories(self):
        e = self.reg.attach("c", "/home/user/inbox/notes draft.txt")
        self.assertEqual(e["redacted_path"], "notes draft.txt")
        e2 = self.reg.attach("c", "folder/")
        self.assertEqual(e2["redacted_path"], "[unnamed]")
        e3 = self.reg.attach("c", "C:\\Users\\neo\\doc.pdf")
        self.assertEqual(e3["redacted_path"], "doc.pdf")

    def test_duplicate_attach_creates_two_refs(self):
        self.reg.attach("c", "/a.txt")
        self.reg.attach("c", "/a.txt")
        self.assertEqual(self.reg.count("c"), 2)

    def test_list_newest_first_with_ties_by_id(self):
        self.reg.attach("c", "/a.txt", now=10)
        self.reg.attach("c", "/b.txt", now=20)
        self.reg.attach("c", "/c.txt", now=20)
        rows = self.reg.list_for("c")
        self.assertEqual([r["path"] for r in rows], ["/b.txt", "/c.txt", "/a.txt"])

    def test_list_excludes_removed_unless_asked(self):
        a = self.reg.attach("c", "/a.txt")
        self.reg.attach("c", "/b.txt")
        self.reg.remove(a["id"])
        self.assertEqual([r["path"] for r in self.reg.list_for("c")], ["/b.txt"])
        self.assertEqual(len(self.reg.list_for("c", include_removed=True)), 2)

    def test_remove_unknown_raises(self):
        with self.assertRaises(KeyError):
            self.reg.remove("att-999999")

    def test_remove_twice_returns_false_and_keeps_record(self):
        a = self.reg.attach("c", "/a.txt")
        self.assertTrue(self.reg.remove(a["id"]))
        self.assertFalse(self.reg.remove(a["id"]))
        self.assertTrue(self.reg.get(a["id"])["removed"])

    def test_get_unknown_raises(self):
        with self.assertRaises(KeyError):
            self.reg.get("att-999999")

    def test_search_case_insensitive_over_path_and_kind(self):
        self.reg.attach("c", "/Home/Report.PDF", kind="invoice")
        self.reg.attach("c", "/x.txt")
        self.assertEqual([r["path"] for r in self.reg.search("c", "report")], ["/Home/Report.PDF"])
        self.assertEqual(len(self.reg.search("c", "invoice")), 1)
        self.assertEqual(self.reg.search("c", "zzz"), [])

    def test_search_empty_query_returns_empty(self):
        self.reg.attach("c", "/a.txt")
        self.assertEqual(self.reg.search("c", ""), [])
        self.assertEqual(self.reg.search("c", "   "), [])

    def test_search_excludes_removed(self):
        a = self.reg.attach("c", "/gone.txt")
        self.reg.attach("c", "/here.txt")
        self.reg.remove(a["id"])
        self.assertEqual([r["path"] for r in self.reg.search("c", "txt")], ["/here.txt"])

    def test_conversations_sorted_and_excludes_removed_only(self):
        self.reg.attach("b-conv", "/a.txt")
        a = self.reg.attach("a-conv", "/b.txt")
        self.assertEqual(self.reg.conversations(), ["a-conv", "b-conv"])
        self.reg.remove(a["id"])
        self.assertEqual(self.reg.conversations(), ["b-conv"])

    def test_count_variants(self):
        self.reg.attach("c1", "/a.txt")
        self.reg.attach("c2", "/b.txt")
        a = self.reg.attach("c1", "/c.txt")
        self.reg.remove(a["id"])
        self.assertEqual(self.reg.count(), 2)
        self.assertEqual(self.reg.count("c1"), 1)
        self.assertEqual(self.reg.count("c1", include_removed=True), 2)

    def test_non_callable_clock_rejected(self):
        with self.assertRaises(TypeError):
            AttachmentRegistry("not-callable")


if __name__ == "__main__":
    unittest.main()
