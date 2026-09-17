"""Workspace layout templates: save, list, apply named layout templates.

Pure data-plane module for the F10 spike. It deals only with layout data:

* a strict layout schema ``{dashboards: [...], pins: [...]}``
* a :class:`TemplateStore` over an injected backing store that exposes
  ``read()`` and ``write(text)`` (protocol style)
* deterministic JSON serialize/deserialize helpers

No environment, clock, network, or ambient filesystem access happens here.
``created_at`` is injected by callers; the only filesystem-touching class is
the optional :class:`JsonFileStore` convenience wrapper, which the pure tests
never instantiate.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any, Dict, Iterable, List, Mapping, Optional, Protocol, runtime_checkable

__all__ = [
    "TemplateValidationError",
    "TemplateNotFoundError",
    "LAYOUT_KEYS",
    "validate_layout",
    "serialize_layout",
    "deserialize_layout",
    "BackingStore",
    "JsonFileStore",
    "TemplateRecord",
    "TemplateStore",
]


class TemplateValidationError(ValueError):
    """Raised when a template name or layout fails validation."""


class TemplateNotFoundError(KeyError):
    """Raised when a template name is not present in the store."""


LAYOUT_KEYS = ("dashboards", "pins")


def _require_string_list(value: Any, key: str) -> None:
    if not isinstance(value, list):
        raise TemplateValidationError(
            "layout key %r must be a list, got %s" % (key, type(value).__name__)
        )
    for index, item in enumerate(value):
        if not isinstance(item, Mapping):
            raise TemplateValidationError(
                "layout key %r item %d must be an object, got %s"
                % (key, index, type(item).__name__)
            )


def validate_layout(layout: Any) -> Dict[str, List[Dict[str, Any]]]:
    """Validate a layout mapping and return a normalized deep copy.

    The accepted schema is exactly ``{"dashboards": [...], "pins": [...]}``.
    Both keys are required and each must be a list of mappings. Any unknown
    top-level key raises :class:`TemplateValidationError` naming the key.
    """
    if not isinstance(layout, Mapping):
        raise TemplateValidationError(
            "layout must be a mapping, got %s" % type(layout).__name__
        )

    unknown = [key for key in layout if key not in LAYOUT_KEYS]
    if unknown:
        raise TemplateValidationError("unknown layout key: %s" % unknown[0])

    missing = [key for key in LAYOUT_KEYS if key not in layout]
    if missing:
        raise TemplateValidationError("missing layout key: %s" % missing[0])

    for key in LAYOUT_KEYS:
        _require_string_list(layout[key], key)

    return {
        "dashboards": [dict(item) for item in layout["dashboards"]],
        "pins": [dict(item) for item in layout["pins"]],
    }


def serialize_layout(layout: Mapping[str, Any]) -> str:
    """Serialize a validated layout to JSON with deterministic key order."""
    normalized = validate_layout(layout)
    return json.dumps(normalized, sort_keys=True, separators=(",", ":"))


def deserialize_layout(text: str) -> Dict[str, List[Dict[str, Any]]]:
    """Parse JSON text back into a validated layout mapping."""
    try:
        parsed = json.loads(text)
    except ValueError as exc:
        raise TemplateValidationError("layout JSON is invalid: %s" % exc) from exc
    return validate_layout(parsed)


@runtime_checkable
class BackingStore(Protocol):
    """Minimal persistence protocol: read() -> str, write(text)."""

    def read(self) -> str: ...

    def write(self, text: str) -> None: ...


class JsonFileStore:
    """Thin filesystem-backed BackingStore convenience (not used by tests)."""

    def __init__(self, path: str) -> None:
        self._path = path

    def read(self) -> str:
        with open(self._path, "r", encoding="utf-8") as handle:
            return handle.read()

    def write(self, text: str) -> None:
        with open(self._path, "w", encoding="utf-8") as handle:
            handle.write(text)


@dataclass(frozen=True)
class TemplateRecord:
    """One stored template as returned by :meth:`TemplateStore.list_templates`."""

    name: str
    layout: Dict[str, List[Dict[str, Any]]] = field(repr=False)
    created_at: str


def _validate_name(name: Any) -> str:
    if not isinstance(name, str):
        raise TemplateValidationError(
            "template name must be a string, got %s" % type(name).__name__
        )
    if not 1 <= len(name) <= 64:
        raise TemplateValidationError(
            "template name must be 1-64 characters, got %d" % len(name)
        )
    for char in name:
        if not ("a" <= char <= "z" or "0" <= char <= "9" or char == "-"):
            raise TemplateValidationError(
                "template name %r contains invalid character %r "
                "(allowed: a-z, 0-9, '-')" % (name, char)
            )
    return name


class TemplateStore:
    """Named workspace layout templates over an injected backing store."""

    def __init__(self, backing_store: BackingStore) -> None:
        self._store = backing_store

    # -- internals -----------------------------------------------------

    def _load_all(self) -> Dict[str, Dict[str, Any]]:
        text = self._store.read()
        if not text.strip():
            return {}
        try:
            parsed = json.loads(text)
        except ValueError as exc:
            raise TemplateValidationError("template store JSON is invalid: %s" % exc) from exc
        if not isinstance(parsed, dict):
            raise TemplateValidationError(
                "template store must be a JSON object, got %s" % type(parsed).__name__
            )
        return parsed

    def _persist(self, templates: Mapping[str, Mapping[str, Any]]) -> None:
        self._store.write(
            json.dumps(dict(templates), sort_keys=True, separators=(",", ":"))
        )

    # -- API -----------------------------------------------------------

    def save_template(self, name: str, layout: Mapping[str, Any], created_at: str) -> TemplateRecord:
        """Validate and store ``layout`` under ``name``.

        ``created_at`` is supplied by the caller (no clock access here).
        Saving an existing name overwrites the previous record atomically:
        the backing store sees exactly one write with the complete new state.
        """
        name = _validate_name(name)
        normalized = validate_layout(layout)
        if not isinstance(created_at, str):
            raise TemplateValidationError(
                "created_at must be a string, got %s" % type(created_at).__name__
            )
        templates = self._load_all()
        templates[name] = {"layout": normalized, "created_at": created_at}
        self._persist(templates)
        return TemplateRecord(name=name, layout=normalized, created_at=created_at)

    def list_templates(self) -> List[TemplateRecord]:
        """All stored records, sorted deterministically by name."""
        templates = self._load_all()
        records: List[TemplateRecord] = []
        for name in sorted(templates):
            entry = templates[name]
            records.append(
                TemplateRecord(
                    name=name,
                    layout=validate_layout(entry["layout"]),
                    created_at=entry["created_at"],
                )
            )
        return records

    def apply_template(
        self,
        name: str,
        current_layout: Optional[Mapping[str, Any]] = None,
        *,
        take_dashboards: bool = True,
        take_pins: bool = True,
    ) -> Dict[str, List[Dict[str, Any]]]:
        """Build a NEW layout from the stored template.

        Merge semantics:

        * ``take_dashboards`` (default True): template dashboards replace the
          current layout's dashboards.
        * ``take_pins`` (default True): pins are unioned by pin ``id`` against
          the current layout's pins; on an id collision the template pin wins.

        The stored template and ``current_layout`` are never mutated; a fresh
        layout object is always returned.
        """
        templates = self._load_all()
        if name not in templates:
            raise TemplateNotFoundError("template not found: %r" % name)
        template_layout = validate_layout(templates[name]["layout"])

        if current_layout is None:
            base: Dict[str, List[Dict[str, Any]]] = {"dashboards": [], "pins": []}
        else:
            base = validate_layout(current_layout)

        dashboards: List[Dict[str, Any]]
        if take_dashboards:
            dashboards = [dict(item) for item in template_layout["dashboards"]]
        else:
            dashboards = [dict(item) for item in base["dashboards"]]

        pins_by_id: Dict[Any, Dict[str, Any]] = {}
        if take_pins:
            for pin in template_layout["pins"]:
                pins_by_id[pin.get("id")] = dict(pin)
        # union: current pins fill ids the template did not take/provide;
        # template wins on collision because it was inserted first.
        for pin in base["pins"]:
            pin_id = pin.get("id")
            if pin_id not in pins_by_id:
                pins_by_id[pin_id] = dict(pin)

        return {"dashboards": dashboards, "pins": list(pins_by_id.values())}

    def delete_template(self, name: str) -> None:
        """Remove ``name``; unknown names raise TemplateNotFoundError."""
        templates = self._load_all()
        if name not in templates:
            raise TemplateNotFoundError("template not found: %r" % name)
        del templates[name]
        self._persist(templates)
