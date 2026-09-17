import sys, os; sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import unittest
from forma_runtime.message_threads import ChatThreads


class FakeClock:
    def __init__(self, start=1000.0, step=10.0):
        self.now = start
        self.step = step

    def __call__(self):
        value = self.now
        self.now += self.step
        return value


class FixedClock:
    def __init__(self, value=500.0):
        self.value = value

    def __call__(self):
        return self.value


class TestChatThreads(unittest.TestCase):
    def test_post_starts_thread(self):
        threads = ChatThreads(FakeClock())
        root = threads.post("hello")
        self.assertEqual(root["id"], "msg-000001")
        self.assertEqual(root["thread_id"], "msg-000001")
        self.assertEqual(root["body"], "hello")
        self.assertEqual(root["created_at"], 1000.0)
        self.assertEqual(root["reaction_counts"], {})
        self.assertEqual(root["reply_count"], 0)

    def test_reply_appends_and_bumps_counts(self):
        threads = ChatThreads(FakeClock())
        root = threads.post("root")
        first = threads.reply(root["id"], "first")
        second = threads.reply(root["id"], "second")
        self.assertEqual(first["thread_id"], root["id"])
        self.assertEqual(second["id"], "msg-000003")
        summaries = threads.list_threads()
        self.assertEqual(summaries[0]["reply_count"], 2)
        self.assertEqual(summaries[0]["last_activity_at"], second["created_at"])

    def test_react_increments(self):
        threads = ChatThreads(FakeClock())
        root = threads.post("root")
        updated = threads.react(root["id"], "plusone")
        self.assertEqual(updated["reaction_counts"], {"plusone": 1})
        threads.react(root["id"], "plusone")
        messages = threads.thread_messages(root["id"])
        self.assertEqual(messages[0]["reaction_counts"], {"plusone": 2})

    def test_unreact_decrements(self):
        threads = ChatThreads(FakeClock())
        root = threads.post("root")
        threads.react(root["id"], "heart")
        updated = threads.unreact(root["id"], "heart")
        self.assertEqual(updated["reaction_counts"], {})

    def test_unreact_missing_reaction_raises(self):
        threads = ChatThreads(FakeClock())
        root = threads.post("root")
        with self.assertRaises(ValueError):
            threads.unreact(root["id"], "rocket")

    def test_react_bad_slug_raises(self):
        threads = ChatThreads(FakeClock())
        root = threads.post("root")
        for slug in ("", "x" * 25, "no\u00e9"):
            with self.assertRaises(ValueError):
                threads.react(root["id"], slug)

    def test_delete_leaf_keeps_thread(self):
        threads = ChatThreads(FakeClock())
        root = threads.post("root")
        leaf = threads.reply(root["id"], "leaf")
        threads.delete_message(leaf["id"])
        messages = threads.thread_messages(root["id"])
        self.assertEqual([m["id"] for m in messages], [root["id"]])
        self.assertEqual(threads.list_threads()[0]["reply_count"], 1)

    def test_delete_root_removes_whole_thread(self):
        threads = ChatThreads(FakeClock())
        root = threads.post("root")
        threads.reply(root["id"], "a")
        threads.reply(root["id"], "b")
        threads.delete_message(root["id"])
        self.assertEqual(threads.list_threads(), [])
        with self.assertRaises(KeyError):
            threads.thread_messages(root["id"])

    def test_same_clock_orders_by_id(self):
        threads = ChatThreads(FixedClock(700.0))
        root = threads.post("root")
        threads.reply(root["id"], "one")
        threads.reply(root["id"], "two")
        threads.reply(root["id"], "three")
        messages = threads.thread_messages(root["id"])
        self.assertEqual(
            [m["id"] for m in messages],
            ["msg-000001", "msg-000002", "msg-000003", "msg-000004"],
        )
        for m in messages:
            self.assertEqual(m["created_at"], 700.0)

    def test_list_threads_orders_by_activity_desc(self):
        threads = ChatThreads(FakeClock())
        first = threads.post("first")
        second = threads.post("second")
        threads.reply(first["id"], "bump")
        summaries = threads.list_threads()
        self.assertEqual(
            [s["thread_id"] for s in summaries],
            [first["id"], second["id"]],
        )
        threads.reply(second["id"], "newer")
        summaries = threads.list_threads()
        self.assertEqual(
            [s["thread_id"] for s in summaries],
            [second["id"], first["id"]],
        )

    def test_unknown_ids_raise_keyerror(self):
        threads = ChatThreads(FakeClock())
        with self.assertRaises(KeyError):
            threads.reply("msg-999999", "x")
        with self.assertRaises(KeyError):
            threads.react("msg-999999", "plusone")
        with self.assertRaises(KeyError):
            threads.unreact("msg-999999", "plusone")
        with self.assertRaises(KeyError):
            threads.delete_message("msg-999999")
        with self.assertRaises(KeyError):
            threads.thread_messages("msg-999999")

    def test_returned_records_are_copies(self):
        threads = ChatThreads(FakeClock())
        root = threads.post("root")
        root["reaction_counts"]["plusone"] = 99
        root["reply_count"] = 99
        stored = threads.thread_messages(root["id"])[0]
        self.assertEqual(stored["reaction_counts"], {})
        self.assertEqual(stored["reply_count"], 0)


if __name__ == "__main__":
    unittest.main()
