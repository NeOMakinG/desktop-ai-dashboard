"""Pure in-memory browser history index for Forma's workspace.

Deterministic data plane: no browser, no I/O, no threads, no clock reads.
All time is injected by the caller. A panel can later bind to this index to
show recently visited sites, search across URLs and titles, and list the
most-visited sites. It stores only URLs, titles, and timestamps; there are
no credential-shaped fields by construction.

Identity model: an entry is keyed by its exact URL string. A repeat visit
to the same URL increments ``visit_count`` and refreshes ``last_visit_at``.
Every read returns fresh dict copies, never live records.
"""

from __future__ import annotations

from typing import Dict, List, Optional

MAX_URL_LENGTH = 2048


class HistoryIndex:
    """Frecency-ranked index of visited URLs with deterministic ordering."""

    def __init__(self, clock) -> None:
        self._clock = clock
        self._entries: Dict[str, Dict[str, object]] = {}
        self._next_number = 1

    # -- recording ---------------------------------------------------------

    def record_visit(
        self, url: str, title: str = "", now: Optional[float] = None
    ) -> Dict[str, object]:
        """Record one visit, creating or refreshing the entry for ``url``."""
        if not isinstance(url, str) or not url.strip():
            raise ValueError("url must be a non-empty string")
        if len(url) > MAX_URL_LENGTH:
            raise ValueError("url longer than %d characters" % MAX_URL_LENGTH)
        timestamp = float(now) if now is not None else float(self._clock())
        entry = self._entries.get(url)
        if entry is None:
            entry = {
                "id": "hist-%06d" % self._next_number,
                "url": url,
                "title": "",
                "visit_count": 0,
                "first_visit_at": timestamp,
                "last_visit_at": timestamp,
            }
            self._next_number += 1
            self._entries[url] = entry
        if title:
            entry["title"] = title
        entry["visit_count"] = entry["visit_count"] + 1
        entry["last_visit_at"] = timestamp
        return dict(entry)

    # -- reads -------------------------------------------------------------

    def search(
        self, query: str, limit: Optional[int] = 20
    ) -> List[Dict[str, object]]:
        """Case-insensitive substring search over url and title.

        Ranking: visit_count desc, then last_visit_at desc, then id asc.
        Empty or whitespace-only queries return an empty list.
        """
        if limit is None:
            limit = 20
        if limit < 1:
            raise ValueError("limit must be at least 1")
        needle = (query or "").strip().lower()
        if not needle:
            return []
        matches = [
            entry
            for entry in self._entries.values()
            if needle in entry["url"].lower()
            or needle in str(entry["title"]).lower()
        ]
        ranked = sorted(
            matches,
            key=lambda e: (-e["visit_count"], -e["last_visit_at"], e["id"]),
        )
        return [dict(entry) for entry in ranked[:limit]]

    def top_sites(self, limit: Optional[int] = 10) -> List[Dict[str, object]]:
        """Most-visited entries; ties broken by recency then id."""
        if limit is None:
            limit = 10
        if limit < 1:
            raise ValueError("limit must be at least 1")
        ranked = sorted(
            self._entries.values(),
            key=lambda e: (-e["visit_count"], -e["last_visit_at"], e["id"]),
        )
        return [dict(entry) for entry in ranked[:limit]]

    def recent(self, limit: Optional[int] = 20) -> List[Dict[str, object]]:
        """Entries ordered by most recent visit; ties broken by id asc."""
        if limit is None:
            limit = 20
        if limit < 1:
            raise ValueError("limit must be at least 1")
        ranked = sorted(
            self._entries.values(),
            key=lambda e: (-e["last_visit_at"], e["id"]),
        )
        return [dict(entry) for entry in ranked[:limit]]

    def get_entry(self, url: str) -> Optional[Dict[str, object]]:
        """Fresh copy of the entry for ``url``, or None when absent."""
        entry = self._entries.get(url)
        return dict(entry) if entry is not None else None

    def forget(self, url: str) -> bool:
        """Remove the entry for ``url``; True when it existed."""
        return self._entries.pop(url, None) is not None

    def entry_count(self) -> int:
        """Number of distinct URLs tracked."""
        return len(self._entries)
