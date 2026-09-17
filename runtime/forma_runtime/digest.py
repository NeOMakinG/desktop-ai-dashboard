"""Daily workspace activity digest (pure in-memory module).

build_digest(events, now) builds a deterministic digest structure from an
injected list of workspace event dicts and an injected `now` (epoch seconds).

Window semantics: only events with ``now - 86400 < ts <= now`` are included.
The lower boundary is EXCLUSIVE: an event exactly 24 hours old (ts equal to
``now - 86400``) is excluded from the digest. The upper boundary is INCLUSIVE:
an event exactly at `now` is included.

Purity: this module performs zero I/O, reads no clocks, and uses no
randomness. All inputs (events and `now`) are injected by the caller.

Grouping: the five known event types are 'message', 'pin', 'export',
'tab_open', and 'command'. Any other 'type' value groups under 'other'.
Event types with zero events in the window are omitted entirely.

Ordering: within each group, items are ordered by (ts, original index)
ascending so equal timestamps preserve injection order deterministically.
Groups are ordered by count descending, then type name ascending.
"""

from datetime import datetime, timezone

KNOWN_TYPES = frozenset({"message", "pin", "export", "tab_open", "command"})

_WINDOW_SECONDS = 86400


def build_digest(events, now):
    """Build a digest dict from injected events and an injected `now`.

    Parameters
    ----------
    events : list of dict
        Each event has keys 'type' (str), 'ts' (epoch seconds), and
        'payload' (str).
    now : int | float
        Current time in epoch seconds, injected by the caller.

    Returns
    -------
    dict
        {'date': ISO date of now (UTC), 'generated_at': now,
         'groups': [{'type': str, 'count': int,
                     'items': [{'ts': ..., 'payload': ...}, ...]}, ...]}
    """
    lower_bound = now - _WINDOW_SECONDS
    buckets = {}

    for index, event in enumerate(events):
        ts = event["ts"]
        if not (lower_bound < ts <= now):
            continue
        event_type = event["type"]
        if event_type not in KNOWN_TYPES:
            event_type = "other"
        buckets.setdefault(event_type, []).append((ts, index, event["payload"]))

    groups = []
    for event_type, entries in buckets.items():
        entries.sort(key=lambda entry: (entry[0], entry[1]))
        groups.append({
            "type": event_type,
            "count": len(entries),
            "items": [
                {"ts": ts, "payload": payload}
                for ts, _index, payload in entries
            ],
        })

    groups.sort(key=lambda group: (-group["count"], group["type"]))

    date_iso = datetime.fromtimestamp(now, tz=timezone.utc).date().isoformat()
    return {
        "date": date_iso,
        "generated_at": now,
        "groups": groups,
    }


def format_digest_text(digest):
    """Render a digest as a compact deterministic multi-line string.

    Each group emits one header line '### <type> (N)' followed by one line per
    item formatted as ' - HH:MM <payload>' with HH:MM derived from the event
    ts in UTC.
    """
    lines = []
    for group in digest["groups"]:
        lines.append("### {} ({})".format(group["type"], group["count"]))
        for item in group["items"]:
            hhmm = datetime.fromtimestamp(item["ts"], tz=timezone.utc).strftime("%H:%M")
            lines.append(" - {} {}".format(hhmm, item["payload"]))
    return "\n".join(lines)
