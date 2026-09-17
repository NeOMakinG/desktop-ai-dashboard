"""Pure in-memory clipboard history data plane for the workspace.

Follows the injected-clock style of reading_list.py, downloads.py and
message_threads.py: no I/O, no threads, fully deterministic, dict-in and
dict-out defensive copies.
"""

MAX_TEXT_LENGTH = 100000


class ClipboardHistory:
    """Deterministic clipboard history store with an injectable clock."""

    def __init__(self, clock):
        if not callable(clock):
            raise TypeError("clock must be callable")
        self._clock = clock
        self._entries = []
        self._sequence = 0

    def _next_id(self):
        self._sequence += 1
        return "clip-%06d" % self._sequence

    @staticmethod
    def _copy_entry(entry):
        return {
            "id": entry["id"],
            "text": entry["text"],
            "created_at": entry["created_at"],
            "pinned": entry["pinned"],
        }

    def _ordered(self, entries):
        pinned = [e for e in entries if e["pinned"]]
        unpinned = [e for e in entries if not e["pinned"]]
        key = lambda e: (e["created_at"], e["id"])
        return sorted(pinned, key=key, reverse=True) + sorted(
            unpinned, key=key, reverse=True
        )

    def copy(self, text, now=None):
        if not isinstance(text, str):
            raise ValueError("text must be a string")
        if text == "":
            raise ValueError("text must not be empty")
        if len(text) > MAX_TEXT_LENGTH:
            raise ValueError("text exceeds maximum length")
        timestamp = self._clock() if now is None else now
        if self._entries and self._entries[-1]["text"] == text:
            newest = self._entries[-1]
            newest["created_at"] = timestamp
            return self._copy_entry(newest)
        entry = {
            "id": self._next_id(),
            "text": text,
            "created_at": timestamp,
            "pinned": False,
        }
        self._entries.append(entry)
        return self._copy_entry(entry)

    def list_entries(self, limit=None, include_unpinned_only=False):
        pool = (
            [e for e in self._entries if not e["pinned"]]
            if include_unpinned_only
            else list(self._entries)
        )
        ordered = self._ordered(pool)
        if limit is not None:
            ordered = ordered[:limit]
        return [self._copy_entry(e) for e in ordered]

    def _require(self, entry_id):
        for entry in self._entries:
            if entry["id"] == entry_id:
                return entry
        raise KeyError(entry_id)

    def pin(self, entry_id):
        entry = self._require(entry_id)
        entry["pinned"] = True
        return self._copy_entry(entry)

    def unpin(self, entry_id):
        entry = self._require(entry_id)
        entry["pinned"] = False
        return self._copy_entry(entry)

    def delete(self, entry_id):
        self._require(entry_id)
        self._entries = [e for e in self._entries if e["id"] != entry_id]

    def search(self, substring):
        needle = substring.lower()
        matches = [e for e in self._entries if needle in e["text"].lower()]
        return [self._copy_entry(e) for e in self._ordered(matches)]

    def to_dict(self):
        return {"entries": self.list_entries()}
