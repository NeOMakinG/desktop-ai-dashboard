"""Pure in-memory chat attachment registry for Forma's workspace.

Stores opaque references (paths) the user attached to a conversation.
Nothing here touches the filesystem: paths are opaque strings, never
opened, resolved, or validated against disk. No I/O, no threads, dict
copies out, deterministic ordering - in the style of the existing
reading_list.py / clipboard_history.py / downloads.py modules.
"""

MAX_PATH_LENGTH = 4096
UNNAMED_LABEL = "[unnamed]"


def _resolve_now(clock, now):
    return clock() if now is None else now


def _redact_path(path):
    normalized = path.replace("\\", "/")
    name = normalized.rsplit("/", 1)[-1]
    return name if name else UNNAMED_LABEL


class AttachmentRegistry:
    def __init__(self, clock):
        if not callable(clock):
            raise TypeError("clock must be callable")
        self._clock = clock
        self._entries = {}
        self._seq = 0

    def _next_id(self):
        self._seq += 1
        return "att-%06d" % self._seq

    def attach(self, conversation_id, path, kind="", now=None):
        if not isinstance(conversation_id, str) or not conversation_id.strip():
            raise ValueError("conversation_id must be a non-empty string")
        if not isinstance(path, str) or not path.strip():
            raise ValueError("path must be a non-empty string")
        if len(path) > MAX_PATH_LENGTH:
            raise ValueError("path too long")
        if not isinstance(kind, str):
            raise ValueError("kind must be a string")
        entry = {
            "id": self._next_id(),
            "conversation_id": conversation_id,
            "path": path,
            "redacted_path": _redact_path(path),
            "kind": kind,
            "attached_at": _resolve_now(self._clock, now),
            "removed": False,
        }
        self._entries[entry["id"]] = entry
        return dict(entry)

    def _ordered(self, conversation_id, include_removed):
        rows = [
            e for e in self._entries.values()
            if (include_removed or not e["removed"])
            and (conversation_id is None or e["conversation_id"] == conversation_id)
        ]
        rows.sort(key=lambda e: (-e["attached_at"], e["id"]))
        return [dict(e) for e in rows]

    def list_for(self, conversation_id, include_removed=False):
        if not isinstance(conversation_id, str) or not conversation_id.strip():
            raise ValueError("conversation_id must be a non-empty string")
        return self._ordered(conversation_id, include_removed)

    def get(self, attachment_id):
        entry = self._entries.get(attachment_id)
        if entry is None:
            raise KeyError(attachment_id)
        return dict(entry)

    def remove(self, attachment_id):
        entry = self._entries.get(attachment_id)
        if entry is None:
            raise KeyError(attachment_id)
        if entry["removed"]:
            return False
        entry["removed"] = True
        return True

    def search(self, conversation_id, substring):
        if not isinstance(substring, str) or not substring.strip():
            return []
        needle = substring.casefold()
        rows = self.list_for(conversation_id)
        return [
            e for e in rows
            if needle in e["path"].casefold() or needle in e["kind"].casefold()
        ]

    def conversations(self):
        return sorted({
            e["conversation_id"] for e in self._entries.values() if not e["removed"]
        })

    def count(self, conversation_id=None, include_removed=False):
        return len(self._ordered(conversation_id, include_removed))
