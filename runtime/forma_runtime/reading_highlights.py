"""Reading-list highlights: anchored, colored annotations on saved entries.

Pure domain module (no I/O, clock-injected), same contract as the other
forma_runtime data planes. Persistence is the caller's job.
"""

import copy

PALETTE = ("yellow", "green", "blue", "pink", "purple")
MAX_TEXT_CHARS = 2000
MAX_NOTE_CHARS = 2000
MAX_CONTEXT_CHARS = 200


def _now(clock, now):
    if now is not None:
        return now
    return clock()


class HighlightStore:
    """Colored highlights plus notes attached to reading-list entries."""

    def __init__(self, clock):
        self._clock = clock
        self._items = []
        self._seq = 0

    # -- creation ---------------------------------------------------
    def add_highlight(self, entry_id, text, note="", color="yellow",
                      anchor=None, now=None):
        self._check_text(text)
        self._check_note(note)
        if color not in PALETTE:
            raise ValueError("unknown highlight color: %r" % (color,))
        anchor = self._check_anchor(anchor)
        stamp = _now(self._clock, now)
        if anchor is not None and self._find_dup(entry_id, anchor, text):
            raise ValueError("duplicate highlight at the same anchor")
        self._seq += 1
        item = {
            "id": "hl-%04d" % self._seq,
            "entry_id": entry_id,
            "text": text,
            "note": note,
            "color": color,
            "anchor": anchor,
            "resolved": False,
            "removed": False,
            "created_at": stamp,
            "updated_at": stamp,
        }
        self._items.append(item)
        return copy.deepcopy(item)

    # -- mutation ---------------------------------------------------
    def update_highlight(self, highlight_id, text=None, note=None, color=None):
        item = self._get_live(highlight_id)
        if text is not None:
            self._check_text(text)
            item["text"] = text
        if note is not None:
            self._check_note(note)
            item["note"] = note
        if color is not None:
            if color not in PALETTE:
                raise ValueError("unknown highlight color: %r" % (color,))
            item["color"] = color
        item["updated_at"] = _now(self._clock, None)
        return copy.deepcopy(item)

    def set_resolved(self, highlight_id, resolved=True):
        item = self._get_live(highlight_id)
        item["resolved"] = bool(resolved)
        item["updated_at"] = _now(self._clock, None)
        return copy.deepcopy(item)

    def delete(self, highlight_id):
        item = self._get_live(highlight_id)
        item["removed"] = True
        item["updated_at"] = _now(self._clock, None)
        return copy.deepcopy(item)

    # -- reads ------------------------------------------------------
    def get(self, highlight_id, include_removed=False):
        for item in self._items:
            if item["id"] == highlight_id:
                if item["removed"] and not include_removed:
                    break
                return copy.deepcopy(item)
        raise KeyError("unknown highlight: %s" % highlight_id)

    def list_highlights(self, entry_id=None, color=None, resolved=None,
                        include_removed=False):
        rows = [i for i in self._items if include_removed or not i["removed"]]
        if entry_id is not None:
            rows = [i for i in rows if i["entry_id"] == entry_id]
        if color is not None:
            rows = [i for i in rows if i["color"] == color]
        if resolved is not None:
            rows = [i for i in rows if i["resolved"] == bool(resolved)]
        rows.sort(key=self._order_key)
        return copy.deepcopy(rows)

    def search(self, query, include_removed=False):
        needle = (query or "").strip().lower()
        if not needle:
            return []
        rows = [i for i in self._items
                if include_removed or not i["removed"]]
        rows = [i for i in rows
                if needle in i["text"].lower() or needle in i["note"].lower()]
        rows.sort(key=self._order_key)
        return copy.deepcopy(rows)

    def counts(self, entry_id=None):
        rows = [i for i in self._items if not i["removed"]]
        if entry_id is not None:
            rows = [i for i in rows if i["entry_id"] == entry_id]
        resolved = len([i for i in rows if i["resolved"]])
        return {"total": len(rows), "resolved": resolved,
                "open": len(rows) - resolved}

    def to_dict(self):
        return {"highlights": copy.deepcopy(self._items)}

    # -- internals --------------------------------------------------
    def _get_live(self, highlight_id):
        for item in self._items:
            if item["id"] == highlight_id and not item["removed"]:
                return item
        raise KeyError("unknown highlight: %s" % highlight_id)

    def _order_key(self, item):
        offset = item["anchor"]["offset"] if item["anchor"] else -1
        return (item["entry_id"], offset, item["created_at"], item["id"])

    def _find_dup(self, entry_id, anchor, text):
        for item in self._items:
            if item["removed"] or item["entry_id"] != entry_id:
                continue
            if not item["anchor"] or item["anchor"]["offset"] != anchor["offset"]:
                continue
            if item["text"] == text:
                return item
        return None

    def _check_text(self, text):
        if not isinstance(text, str) or not text.strip():
            raise ValueError("highlight text must be a non-empty string")
        if len(text) > MAX_TEXT_CHARS:
            raise ValueError("highlight text exceeds %d chars" % MAX_TEXT_CHARS)

    def _check_note(self, note):
        if not isinstance(note, str):
            raise ValueError("note must be a string")
        if len(note) > MAX_NOTE_CHARS:
            raise ValueError("note exceeds %d chars" % MAX_NOTE_CHARS)

    def _check_anchor(self, anchor):
        if anchor is None:
            return None
        if not isinstance(anchor, dict):
            raise ValueError("anchor must be a dict or None")
        offset = anchor.get("offset")
        if not isinstance(offset, int) or isinstance(offset, bool) or offset < 0:
            raise ValueError("anchor.offset must be a non-negative int")
        for side in ("prefix", "suffix"):
            ctx = anchor.get(side, "")
            if not isinstance(ctx, str):
                raise ValueError("anchor.%s must be a string" % side)
            if len(ctx) > MAX_CONTEXT_CHARS:
                raise ValueError("anchor.%s exceeds %d chars" % (side, MAX_CONTEXT_CHARS))
        clean = {"offset": offset}
        if "prefix" in anchor:
            clean["prefix"] = anchor["prefix"]
        if "suffix" in anchor:
            clean["suffix"] = anchor["suffix"]
        return clean
