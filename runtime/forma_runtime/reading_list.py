"""Pure in-memory reading list store for saved pages.

Follows the injected-clock style of the digest, tab_sessions and
quick_notes modules: no I/O, no threads, deterministic ordering.
"""


class ReadingListStore:
    """In-memory store of pages saved to read later."""

    def __init__(self, clock):
        if not callable(clock):
            raise TypeError("clock must be callable")
        self._clock = clock
        self._entries = {}
        self._next_id = 1

    def _get(self, entry_id):
        try:
            return self._entries[entry_id]
        except KeyError:
            raise KeyError("unknown entry id: %r" % (entry_id,))

    def add_entry(self, url, title, note="", now=None):
        created_at = self._clock() if now is None else now
        entry_id = "rl-%06d" % self._next_id
        self._next_id += 1
        entry = {
            "id": entry_id,
            "url": url,
            "title": title,
            "note": note,
            "created_at": created_at,
            "read": False,
            "archived": False,
        }
        self._entries[entry_id] = entry
        return dict(entry)

    def list_entries(self, include_archived=False):
        entries = [
            e for e in self._entries.values()
            if include_archived or not e["archived"]
        ]
        entries.sort(key=lambda e: (e["read"], -e["created_at"], e["id"]))
        return [dict(e) for e in entries]

    def mark_read(self, entry_id):
        self._get(entry_id)["read"] = True

    def mark_unread(self, entry_id):
        self._get(entry_id)["read"] = False

    def archive(self, entry_id):
        self._get(entry_id)["archived"] = True

    def unarchive(self, entry_id):
        self._get(entry_id)["archived"] = False

    def delete(self, entry_id):
        self._get(entry_id)
        del self._entries[entry_id]

    def to_dict(self):
        return {"entries": self.list_entries(include_archived=True)}
