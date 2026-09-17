"""Pure in-memory browser tab session data plane for Forma's embedded browser.

Provides TabSessionStore (save/list/get/delete/group of named tab snapshots)
and a stateless group_tabs_by_domain helper. No I/O, no browser driving,
deterministic ordering throughout.
"""

from urllib.parse import urlsplit


class TabSessionStore:
    """In-memory store of named browser tab session snapshots."""

    def __init__(self, clock):
        self._clock = clock
        self._sessions = {}
        self._counter = 0

    def _next_id(self):
        self._counter += 1
        return "sess-%04d" % self._counter

    def save_session(self, name, tabs):
        sid = self._next_id()
        session = {
            "id": sid,
            "name": name,
            "tabs": [dict(t) for t in tabs],
            "saved_at": self._clock(),
        }
        self._sessions[sid] = session
        return session

    def list_sessions(self):
        return sorted(
            self._sessions.values(),
            key=lambda s: (-s["saved_at"], s["id"]),
        )

    def get_session(self, sid):
        return self._sessions.get(sid)

    def delete_session(self, sid):
        if sid in self._sessions:
            del self._sessions[sid]
            return True
        return False

    def group_sessions(self):
        groups = {}
        for session in self._sessions.values():
            groups.setdefault(session["name"], []).append(session)
        for name in groups:
            groups[name].sort(key=lambda s: (s["saved_at"], s["id"]))
        return groups


def group_tabs_by_domain(tabs):
    """Map hostname -> tab count; unparseable/missing URLs count under 'other'."""
    counts = {}
    for tab in tabs:
        host = None
        try:
            url = tab.get("url") if isinstance(tab, dict) else None
            if isinstance(url, str):
                host = urlsplit(url).hostname
        except ValueError:
            host = None
        key = host if host else "other"
        counts[key] = counts.get(key, 0) + 1
    return counts
