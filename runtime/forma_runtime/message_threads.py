"""Pure in-memory chat message threads and reactions data plane.

Deterministic, dict-in/dict-out, injected clock, no I/O, no threading.
"""


class ChatThreads:
    """Deterministic store of chat threads, messages, and reaction counts."""

    def __init__(self, clock):
        self._clock = clock
        self._messages = {}
        self._threads = {}
        self._counter = 0

    def _next_id(self):
        self._counter += 1
        return "msg-%06d" % self._counter

    def _now(self, now):
        return now if now is not None else self._clock()

    def _copy(self, record):
        out = dict(record)
        out["reaction_counts"] = dict(record["reaction_counts"])
        return out

    def post(self, body, now=None):
        if not isinstance(body, str) or not body:
            raise ValueError("body must be a non-empty string")
        message_id = self._next_id()
        stamp = self._now(now)
        record = {
            "id": message_id,
            "thread_id": message_id,
            "body": body,
            "created_at": stamp,
            "reaction_counts": {},
            "reply_count": 0,
            "last_activity_at": stamp,
        }
        self._messages[message_id] = record
        self._threads[message_id] = {
            "order": [message_id],
            "last_activity_at": stamp,
        }
        return self._copy(record)

    def reply(self, thread_id, body, now=None):
        if thread_id not in self._threads:
            raise KeyError(thread_id)
        if not isinstance(body, str) or not body:
            raise ValueError("body must be a non-empty string")
        message_id = self._next_id()
        stamp = self._now(now)
        record = {
            "id": message_id,
            "thread_id": thread_id,
            "body": body,
            "created_at": stamp,
            "reaction_counts": {},
            "reply_count": 0,
        }
        self._messages[message_id] = record
        thread = self._threads[thread_id]
        thread["order"].append(message_id)
        thread["last_activity_at"] = stamp
        root = self._messages[thread_id]
        root["reply_count"] += 1
        root["last_activity_at"] = stamp
        return self._copy(record)

    def react(self, message_id, slug, now=None):
        if message_id not in self._messages:
            raise KeyError(message_id)
        self._resolve_slug(slug)
        counts = self._messages[message_id]["reaction_counts"]
        counts[slug] = counts.get(slug, 0) + 1
        return self._copy(self._messages[message_id])

    def unreact(self, message_id, slug):
        if message_id not in self._messages:
            raise KeyError(message_id)
        self._resolve_slug(slug)
        counts = self._messages[message_id]["reaction_counts"]
        if slug not in counts:
            raise ValueError("message has no such reaction")
        counts[slug] -= 1
        if counts[slug] <= 0:
            del counts[slug]
        return self._copy(self._messages[message_id])

    def delete_message(self, message_id):
        if message_id not in self._messages:
            raise KeyError(message_id)
        thread_id = self._messages[message_id]["thread_id"]
        if message_id == thread_id:
            for mid in self._threads[thread_id]["order"]:
                del self._messages[mid]
            del self._threads[thread_id]
        else:
            del self._messages[message_id]
            thread = self._threads[thread_id]
            thread["order"].remove(message_id)

    def thread_messages(self, thread_id):
        if thread_id not in self._threads:
            raise KeyError(thread_id)
        records = [self._messages[mid] for mid in self._threads[thread_id]["order"]]
        records.sort(key=lambda r: (r["created_at"], r["id"]))
        return [self._copy(r) for r in records]

    def list_threads(self):
        summaries = []
        for thread_id, thread in self._threads.items():
            root = self._messages[thread_id]
            summaries.append({
                "thread_id": thread_id,
                "body": root["body"],
                "reply_count": root["reply_count"],
                "last_activity_at": thread["last_activity_at"],
            })
        summaries.sort(key=lambda s: (-s["last_activity_at"], s["thread_id"]))
        return summaries

    @staticmethod
    def _resolve_slug(slug):
        if not isinstance(slug, str) or not slug:
            raise ValueError("slug must be a non-empty string")
        if len(slug) > 24:
            raise ValueError("slug must be at most 24 characters")
        if not slug.isascii():
            raise ValueError("slug must be ASCII")
        return slug
