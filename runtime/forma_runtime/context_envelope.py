"""Redacted app-state context envelope for the future `app.state.read` bridge row.

Pure, additive data-plane module (ADR 0004 backlog job: host-owned envelope builder
only; the MCP transport is a separate backlog job and stays out of scope).

`build_app_state_envelope(...)` assembles a bounded, redacted JSON envelope from
explicit inputs:

- workspaces:   {id, title, updated_at}
- interfaces:   {id, name, revision, status}
- grants:       {id, scope, status, expiry}   (never payloads or receipts)
- automation:   counts plus per-item {id, state}

Guarantees:

- Strict allowlist serialization: unknown input fields are dropped, never passed
  through. Message bodies, account payloads, and chat-history fields have no
  accepted field name, so they are structurally impossible to include.
- Credential redaction: keys matching api_key/token/secret/password/authorization/
  cookie (case-insensitive, any nesting depth via dicts and lists) are dropped and
  counted in a `redactions` counter on the result.
- Hard size caps: max serialized bytes (default 8192) and max items per collection;
  exceeding either sets an explicit `truncated: true` flag with retained counts —
  never silent truncation.
- Deterministic output: sorted keys, collections ordered by id, and an injectable
  `now` parameter so two calls with identical inputs (and identical `now`) produce
  byte-identical serialized output. Production callers omit `now` to get real UTC.

No network, no I/O, no clock side effects beyond the injectable timestamp.
"""

from __future__ import annotations

import datetime as _dt
import json

SCHEMA_VERSION = "1"
PROVENANCE = "forma-runtime app.state.read envelope"
DEFAULT_MAX_BYTES = 8192
DEFAULT_MAX_ITEMS = 50

_CREDENTIAL_KEY_MARKERS = (
    "api_key",
    "token",
    "secret",
    "password",
    "authorization",
    "cookie",
)

_WORKSPACE_FIELDS = ("id", "title", "updated_at")
_INTERFACE_FIELDS = ("id", "name", "revision", "status")
_GRANT_FIELDS = ("id", "scope", "status", "expiry")
_AUTOMATION_ITEM_FIELDS = ("id", "state")

_JSON_KWARGS = {
    "sort_keys": True,
    "separators": (",", ":"),
    "ensure_ascii": True,
}


class _RedactionCounter(object):
    def __init__(self):
        self.count = 0


def _is_credential_key(key):
    if not isinstance(key, str):
        return False
    lowered = key.lower()
    return any(marker in lowered for marker in _CREDENTIAL_KEY_MARKERS)


def _redact(value, counter):
    """Deep-copy `value`, dropping credential keys at any dict/list nesting depth."""
    if isinstance(value, dict):
        out = {}
        for key, item in value.items():
            if _is_credential_key(key):
                counter.count += 1
                continue
            out[key] = _redact(item, counter)
        return out
    if isinstance(value, list):
        return [_redact(item, counter) for item in value]
    return value


def _project(item, allowed_fields):
    """Allowlist projection: keep only explicitly permitted top-level fields."""
    if not isinstance(item, dict):
        return None
    projected = {}
    for field in allowed_fields:
        if field in item:
            projected[field] = item[field]
    return projected


def _stable_by_id(items):
    def sort_key(item):
        item_id = item.get("id")
        if isinstance(item_id, (int, float)) and not isinstance(item_id, bool):
            return (0, item_id, "")
        return (1, 0, str(item_id))

    return sorted(items, key=sort_key)


def _serialize(envelope):
    return json.dumps(envelope, **_JSON_KWARGS)


def _byte_len(blob):
    return len(blob.encode("utf-8"))


def build_app_state_envelope(
    workspaces=None,
    interfaces=None,
    grants=None,
    automation=None,
    *,
    max_bytes=DEFAULT_MAX_BYTES,
    max_items=DEFAULT_MAX_ITEMS,
    now=None,
):
    """Build the redacted, allowlist-serialized, size-capped app-state envelope.

    Returns a dict with the envelope header (schema_version, generated_at_utc,
    provenance) plus workspaces/interfaces/grants collections, an automation
    summary (total count plus per-item {id, state}), a `redactions` counter, a
    `truncated` flag, and `retained_counts`. The result is deterministic for
    identical inputs and an identical injected `now` (ISO-8601 UTC, 'Z' suffix).
    """
    counter = _RedactionCounter()

    def prepare(items, allowed_fields):
        prepared = []
        for raw in items or []:
            if not isinstance(raw, dict):
                continue
            projected = _project(raw, allowed_fields)
            if projected is None:
                continue
            prepared.append(_redact(projected, counter))
        return _stable_by_id(prepared)

    workspaces_out = prepare(workspaces, _WORKSPACE_FIELDS)[:max_items]
    interfaces_out = prepare(interfaces, _INTERFACE_FIELDS)[:max_items]
    grants_out = prepare(grants, _GRANT_FIELDS)[:max_items]

    automation_input = automation or {}
    automation_total = automation_input.get("total", 0)
    if not isinstance(automation_total, int) or isinstance(automation_total, bool):
        automation_total = 0
    automation_items_out = prepare(
        automation_input.get("items", []), _AUTOMATION_ITEM_FIELDS
    )[:max_items]

    if now is None:
        now = _dt.datetime.now(_dt.timezone.utc)
    elif isinstance(now, _dt.datetime) and now.tzinfo is not None:
        now = now.astimezone(_dt.timezone.utc)
    generated_at_utc = (
        now.strftime("%Y-%m-%dT%H:%M:%SZ")
        if isinstance(now, _dt.datetime)
        else str(now)
    )

    envelope = {
        "schema_version": SCHEMA_VERSION,
        "generated_at_utc": generated_at_utc,
        "provenance": PROVENANCE,
        "workspaces": workspaces_out,
        "interfaces": interfaces_out,
        "grants": grants_out,
        "automation": {
            "total": automation_total,
            "items": automation_items_out,
        },
        "redactions": counter.count,
        "truncated": False,
        "retained_counts": {
            "workspaces": len(workspaces_out),
            "interfaces": len(interfaces_out),
            "grants": len(grants_out),
            "automation_items": len(automation_items_out),
        },
    }

    item_capped = (
        len(workspaces or []) > len(workspaces_out)
        or len(interfaces or []) > len(interfaces_out)
        or len(grants or []) > len(grants_out)
        or len((automation_input.get("items", []) if isinstance(automation_input, dict) else []))
        > len(automation_items_out)
    )

    blob = _serialize(envelope)
    byte_capped = False
    while _byte_len(blob) > max_bytes:
        byte_capped = True
        dropped = False
        for collection in ("workspaces", "interfaces", "grants"):
            if envelope[collection]:
                envelope[collection].pop()
                dropped = True
                break
        if not dropped and envelope["automation"]["items"]:
            envelope["automation"]["items"].pop()
            dropped = True
        envelope["retained_counts"] = {
            "workspaces": len(envelope["workspaces"]),
            "interfaces": len(envelope["interfaces"]),
            "grants": len(envelope["grants"]),
            "automation_items": len(envelope["automation"]["items"]),
        }
        blob = _serialize(envelope)
        if not dropped:
            break

    envelope["truncated"] = bool(item_capped or byte_capped)
    blob = _serialize(envelope)
    if _byte_len(blob) > max_bytes:
        envelope["truncated"] = True

    return envelope
