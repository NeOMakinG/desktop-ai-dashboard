"""Quick notes scratchpad data plane for Forma workspaces.

Pure-python module following the store-injection pattern of digest.py and
tab_sessions.py: every function takes an injected dict-like store holding
JSON-serializable note records and an injected ``datetime`` for timestamps.
No wall clock and no IO beyond the store.
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass
from datetime import datetime
from typing import Mapping

MAX_NOTE_LENGTH = 500


@dataclass
class QuickNote:
    """A single scratchpad note."""

    id: str
    text: str
    created_at: datetime
    updated_at: datetime
    pinned: bool

    def to_json(self) -> dict:
        """Return a JSON-serializable dict for this note."""
        return {
            "id": self.id,
            "text": self.text,
            "created_at": self.created_at.isoformat(),
            "updated_at": self.updated_at.isoformat(),
            "pinned": self.pinned,
        }

    @classmethod
    def from_json(cls, data: Mapping) -> "QuickNote":
        """Rebuild a QuickNote from its serialized dict form."""
        return cls(
            id=data["id"],
            text=data["text"],
            created_at=datetime.fromisoformat(data["created_at"]),
            updated_at=datetime.fromisoformat(data["updated_at"]),
            pinned=bool(data["pinned"]),
        )


def _new_note_id(store: Mapping) -> str:
    """Generate an id guaranteed not to collide with the given store."""
    while True:
        candidate = uuid.uuid4().hex
        if candidate not in store:
            return candidate


def save_note(store, text: str, now: datetime) -> QuickNote:
    """Validate and save a new note; new notes start unpinned."""
    if not isinstance(text, str) or not text.strip():
        raise ValueError("note text must not be empty or whitespace only")
    if len(text) > MAX_NOTE_LENGTH:
        raise ValueError("note text must not exceed 500 characters")
    note = QuickNote(
        id=_new_note_id(store),
        text=text,
        created_at=now,
        updated_at=now,
        pinned=False,
    )
    store[note.id] = note.to_json()
    return note


def list_notes(store) -> list:
    """Return all notes: pinned first, then updated_at descending,
    with id ascending as the final deterministic tiebreak."""
    notes = [QuickNote.from_json(record) for record in store.values()]
    notes.sort(key=lambda note: note.id)
    notes.sort(key=lambda note: note.updated_at, reverse=True)
    notes.sort(key=lambda note: note.pinned, reverse=True)
    return notes


def _set_pinned(store, note_id: str, pinned: bool, now: datetime) -> QuickNote:
    if note_id not in store:
        raise KeyError(note_id)
    note = QuickNote.from_json(store[note_id])
    note.pinned = pinned
    note.updated_at = now
    store[note.id] = note.to_json()
    return note


def pin_note(store, note_id: str, now: datetime) -> QuickNote:
    """Pin a note and bump its updated_at using the injected now."""
    return _set_pinned(store, note_id, True, now)


def unpin_note(store, note_id: str, now: datetime) -> QuickNote:
    """Unpin a note and bump its updated_at using the injected now."""
    return _set_pinned(store, note_id, False, now)


def delete_note(store, note_id: str) -> None:
    """Remove a note; unknown ids raise KeyError."""
    if note_id not in store:
        raise KeyError(note_id)
    del store[note_id]
