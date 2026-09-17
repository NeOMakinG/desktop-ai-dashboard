"""Pure in-memory downloads data plane.

Tracks download records with an injectable clock, mirroring the style of
digest.py, tab_sessions.py, quick_notes.py and reading_list.py: no I/O,
no threads, fully deterministic, dict-in/dict-out copies.
"""


_ACTIVE = 'active'
_COMPLETED = 'completed'
_FAILED = 'failed'
_CANCELLED = 'cancelled'
_STATES = (_ACTIVE, _COMPLETED, _FAILED, _CANCELLED)


class DownloadManager(object):
    """In-memory manager for download records."""

    def __init__(self, clock):
        if not callable(clock):
            raise TypeError('clock must be callable')
        self._clock = clock
        self._records = {}
        self._next_id = 1

    def _resolve_now(self, now):
        if now is not None:
            return now
        return self._clock()

    @staticmethod
    def _copy(record):
        return dict(record)

    def _lookup(self, download_id):
        try:
            return self._records[download_id]
        except KeyError:
            raise KeyError(download_id)

    def _require_active(self, download_id, action):
        record = self._lookup(download_id)
        if record['state'] != _ACTIVE:
            raise ValueError(
                'cannot %s download %r in state %r'
                % (action, download_id, record['state']))
        return record

    def start_download(self, url, filename, mime=None, now=None):
        """Create a new active download record and return a copy."""
        download_id = 'dl-%06d' % self._next_id
        self._next_id += 1
        record = {
            'id': download_id,
            'url': url,
            'filename': filename,
            'mime': mime,
            'state': _ACTIVE,
            'started_at': self._resolve_now(now),
            'received_bytes': 0,
            'total_bytes': None,
            'finished_at': None,
            'error': None,
        }
        self._records[download_id] = record
        return self._copy(record)

    def update_progress(self, download_id, received_bytes,
                        total_bytes=None, now=None):
        """Advance progress on an active download.

        Only meaningful for active records; terminal or unknown records are
        rejected. When a total is known (newly supplied or previously set),
        received_bytes is clamped to it.
        """
        record = self._require_active(download_id, 'update progress for')
        if total_bytes is not None:
            record['total_bytes'] = total_bytes
        if received_bytes < 0:
            received_bytes = 0
        if record['total_bytes'] is not None and received_bytes > record['total_bytes']:
            received_bytes = record['total_bytes']
        record['received_bytes'] = received_bytes
        return self._copy(record)

    def complete(self, download_id, now=None):
        """Move an active download to completed and return a copy."""
        record = self._require_active(download_id, 'complete')
        record['state'] = _COMPLETED
        record['finished_at'] = self._resolve_now(now)
        return self._copy(record)

    def fail(self, download_id, reason, now=None):
        """Move an active download to failed with an error reason."""
        record = self._require_active(download_id, 'fail')
        record['state'] = _FAILED
        record['finished_at'] = self._resolve_now(now)
        record['error'] = reason
        return self._copy(record)

    def cancel(self, download_id, now=None):
        """Move an active download to cancelled."""
        record = self._require_active(download_id, 'cancel')
        record['state'] = _CANCELLED
        record['finished_at'] = self._resolve_now(now)
        return self._copy(record)

    def remove(self, download_id):
        """Delete a record. Active records are rejected; unknown raise KeyError."""
        record = self._lookup(download_id)
        if record['state'] == _ACTIVE:
            raise ValueError(
                'cannot remove download %r in state %r'
                % (download_id, record['state']))
        del self._records[download_id]

    def list_downloads(self, state=None):
        """Return copies, newest first, with an optional state filter.

        Ordering uses the finished timestamp when present, otherwise the
        started timestamp, descending; equal timestamps resolve by id
        ascending.
        """
        if state is not None and state not in _STATES:
            raise ValueError('unknown state %r' % (state,))
        records = list(self._records.values())
        if state is not None:
            records = [r for r in records if r['state'] == state]
        records.sort(
            key=lambda r: (
                -(r['finished_at'] if r['finished_at'] is not None
                  else r['started_at']),
                r['id'],
            )
        )
        return [self._copy(r) for r in records]

    def stats(self):
        """Return per-state counts, including zero entries."""
        counts = {s: 0 for s in _STATES}
        for record in self._records.values():
            counts[record['state']] += 1
        return counts
