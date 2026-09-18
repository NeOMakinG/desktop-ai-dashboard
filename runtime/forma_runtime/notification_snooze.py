"""Pure in-memory notification snooze store for Forma.

Users can silence a notification rule for a while instead of disabling
it forever. This module stores those snoozes and answers two questions
for the rest of the app: which snoozes have woken up (due), and which
are still sleeping (pending). Nothing here performs I/O, reads a real
clock, uses threads, or touches the filesystem: the clock is injected,
dict copies go out, ordering is deterministic - in the style of the
existing notification_rules.py / digest.py / attachments.py modules.

Dedupe rule: one live snooze per (rule_id, minutes) pair. Snoozing a
pair that is already asleep is a no-op (existing row back, created
False); snoozing an already-awake or cancelled pair re-arms it in
place (same id, created True). A rule/duration pair therefore shows up
at most once across due() and pending().
"""

from __future__ import annotations

from typing import Dict, List, Optional, Tuple

MIN_MINUTES = 1
MAX_MINUTES = 43200  # 30 days
MAX_REASON_CHARS = 500


def _resolve_now(clock, now):
    return clock() if now is None else now


class SnoozeStore:
    """Store and evaluate snoozes placed on notification rule ids."""

    def __init__(self, clock) -> None:
        if not callable(clock):
            raise TypeError("clock must be callable")
        self._clock = clock
        self._snoozes: Dict[Tuple[str, int], Dict[str, object]] = {}
        self._seq = 0

    def _next_id(self) -> str:
        self._seq += 1
        return "snooze-%06d" % self._seq

    def snooze(
        self,
        rule_id: str,
        minutes: int,
        reason: str = "",
        now: Optional[float] = None,
    ) -> Tuple[Dict[str, object], bool]:
        """Snooze one rule for `minutes`; returns (row, created).

        created is False only when an identical live pair already
        sleeps past `now`; that returns the existing row unchanged.
        Every other case creates or re-arms the pair (same id when a
        row exists) and returns created=True.
        """
        if not isinstance(rule_id, str) or not rule_id.strip():
            raise ValueError("rule_id must be a non-empty string")
        if not isinstance(minutes, int) or isinstance(minutes, bool):
            raise ValueError("minutes must be an integer")
        if not MIN_MINUTES <= minutes <= MAX_MINUTES:
            raise ValueError(
                "minutes must be between %d and %d" % (MIN_MINUTES, MAX_MINUTES)
            )
        if not isinstance(reason, str):
            raise ValueError("reason must be a string")
        if len(reason) > MAX_REASON_CHARS:
            raise ValueError(
                "reason must be at most %d characters" % MAX_REASON_CHARS
            )
        ts = _resolve_now(self._clock, now)
        key = (rule_id, minutes)
        existing = self._snoozes.get(key)
        if (
            existing is not None
            and not existing["removed"]
            and existing["wake_at"] > ts
        ):
            return dict(existing), False
        if existing is None:
            snooze_id = self._next_id()
        else:
            snooze_id = existing["id"]
        row = {
            "id": snooze_id,
            "rule_id": rule_id,
            "minutes": minutes,
            "reason": reason,
            "created_at": ts,
            "wake_at": ts + minutes * 60,
            "removed": False,
        }
        self._snoozes[key] = row
        return dict(row), True

    def _find(self, snooze_id: str) -> Optional[Dict[str, object]]:
        for row in self._snoozes.values():
            if row["id"] == snooze_id:
                return row
        return None

    def cancel(self, snooze_id: str) -> bool:
        """Cancel a live snooze. False when already cancelled."""
        row = self._find(snooze_id)
        if row is None:
            raise KeyError(snooze_id)
        if row["removed"]:
            return False
        row["removed"] = True
        return True

    def get(self, snooze_id: str) -> Dict[str, object]:
        """Return a copy of one live snooze; KeyError if missing/cancelled."""
        row = self._find(snooze_id)
        if row is None or row["removed"]:
            raise KeyError(snooze_id)
        return dict(row)

    def due(self, now: Optional[float] = None) -> List[Dict[str, object]]:
        """Live snoozes whose wake time has arrived, ordered (wake_at, id)."""
        ts = _resolve_now(self._clock, now)
        rows = [
            r for r in self._snoozes.values()
            if not r["removed"] and r["wake_at"] <= ts
        ]
        rows.sort(key=lambda r: (r["wake_at"], r["id"]))
        return [dict(r) for r in rows]

    def pending(
        self,
        rule_id: Optional[str] = None,
        now: Optional[float] = None,
    ) -> List[Dict[str, object]]:
        """Live snoozes still sleeping, ordered (wake_at, id).

        With rule_id, only that rule's sleeping snoozes are returned.
        """
        ts = _resolve_now(self._clock, now)
        rows = [
            r for r in self._snoozes.values()
            if not r["removed"]
            and r["wake_at"] > ts
            and (rule_id is None or r["rule_id"] == rule_id)
        ]
        rows.sort(key=lambda r: (r["wake_at"], r["id"]))
        return [dict(r) for r in rows]
