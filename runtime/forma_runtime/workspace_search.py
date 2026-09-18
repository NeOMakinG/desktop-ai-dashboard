"""Pure in-memory workspace-wide search index for Forma.

Deterministic data plane for the workspace search bar: register documents
from any source (chat transcripts, notes, dashboards, tabs), then search
across all of them with stable deterministic ordering. No I/O, no threads,
no clock reads — all time is injected by the caller. Reads return fresh
dict copies, never live records.

Cross-source registers: no document type is registered twice, so two
sources of the same kind unify onto one entry (the last register wins) —
a deliberate dedupe rule so the results list stays clean.
"""

from __future__ import annotations

from typing import Dict, List, Optional


class WorkspaceSearch:
    """Unified cross-source search over registered workspace documents."""

    def __init__(self, clock) -> None:
        self._clock = clock
        self._docs: Dict[tuple, Dict[str, object]] = {}
        self._next_number = 1

    def register(
        self,
        source: str,
        key: str,
        title: str,
        text: str = "",
        url: str = "",
        now: Optional[float] = None,
    ) -> Dict[str, object]:
        """Register (or replace) one searchable document.

        Identity is the (source, key) pair. Re-registering the same pair
        replaces the stored text and metadata, refreshes registered_at, and
        keeps the stable id, so UI list keys survive a refresh.
        """
        if not isinstance(source, str) or not source.strip():
            raise ValueError("source must be a non-empty string")
        if not isinstance(key, str) or not key:
            raise ValueError("key must be a non-empty string")
        if not isinstance(title, str) or not title.strip():
            raise ValueError("title must be a non-empty string")
        if not isinstance(text, str):
            raise ValueError("text must be a string")
        timestamp = float(now) if now is not None else float(self._clock())
        existing = self._docs.get((source, key))
        if existing is not None:
            doc_id = existing["id"]
        else:
            doc_id = "doc-%06d" % self._next_number
            self._next_number += 1
        doc = {
            "id": doc_id,
            "source": source,
            "key": key,
            "title": title,
            "text": text,
            "url": url,
            "registered_at": timestamp,
        }
        self._docs[(source, key)] = doc
        return dict(doc)

    def search(
        self, query: str, limit: Optional[int] = 20
    ) -> List[Dict[str, object]]:
        """Rank matches across title, text, and url.

        Ordering: title-match desc, source asc, then id asc. A title match
        always outranks a text-only match, so names beat contents. Empty or
        whitespace-only queries return an empty list.
        """
        if limit is None:
            limit = 20
        if limit < 1:
            raise ValueError("limit must be at least 1")
        needle = (query or "").strip().lower()
        if not needle:
            return []
        matches = []
        for doc_id in sorted(self._docs):
            doc = self._docs[doc_id]
            in_title = needle in doc["title"].lower()
            in_text = needle in doc["text"].lower() or needle in doc["url"].lower()
            if in_title or in_text:
                matches.append((in_title, doc))
        matches.sort(key=lambda pair: (not pair[0], pair[1]["source"], pair[1]["id"]))
        return [dict(doc) for _, doc in matches[:limit]]

    def sources(self) -> List[str]:
        """Sorted distinct sources currently registered."""
        return sorted({doc["source"] for doc in self._docs.values()})

    def count(self) -> int:
        """Total number of registered documents."""
        return len(self._docs)

    def clear_source(self, source: str) -> int:
        """Drop all documents of one source; returns how many were removed."""
        if not isinstance(source, str) or not source:
            raise ValueError("source must be a non-empty string")
        doomed = [k for k in self._docs if k[0] == source]
        for k in doomed:
            del self._docs[k]
        return len(doomed)
