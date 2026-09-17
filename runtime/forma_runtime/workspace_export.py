"""Pure data-plane workspace export to Markdown.

Renders a workspace (messages in reading order, remembered-context notes,
interface references) as a deterministic Markdown transcript and validates
export requests. This module performs NO I/O: the future chat tool owns the
actual file write through approved runtime channels and must re-validate the
path at write time (symlink recheck). All functions are pure; exported_at is
an injected parameter, never a clock read.
"""

from __future__ import annotations

import os
import re
from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional, Sequence


class ExportError(ValueError):
    """Base typed error for workspace export validation failures."""


class ExportWorkspaceError(ExportError):
    """Raised when workspace identity is invalid."""


class ExportPathError(ExportError):
    """Raised when the requested export path is invalid or unsafe."""


_CREDENTIAL_PATTERNS: List[re.Pattern[str]] = [
    re.compile(r"(?i)bearer\s+[A-Za-z0-9\-._~+/]+=*"),
    re.compile(r"(?i)\b(?:api[_-]?key|apikey)\s*[:=]\s*[A-Za-z0-9\-_.]{16,}"),
    re.compile(r"\bsk-[A-Za-z0-9\-_.]{16,}\b"),
    re.compile(r"(?i)\b(?:token|secret|password)\s*[:=]\s*[A-Za-z0-9\-_.]{16,}"),
]

_REDACTED = "[REDACTED]"


def redact(text: str) -> str:
    """Replace stored-credential-shaped substrings with [REDACTED].

    Pattern set is intentionally extendable; it covers common bearer tokens
    and api-key shapes and makes no claim of exhaustive secret detection.
    """
    if not text:
        return text
    for pattern in _CREDENTIAL_PATTERNS:
        text = pattern.sub(_REDACTED, text)
    return text


@dataclass(frozen=True)
class ExportRequest:
    """Validated export request. path is optional (None means caller chooses)."""

    workspace_id: str
    workspace_name: str
    path: Optional[str]

    @classmethod
    def create(
        cls,
        workspace_id: str,
        workspace_name: str,
        path: Optional[str] = None,
        home_dir: Optional[str] = None,
    ) -> "ExportRequest":
        if not isinstance(workspace_id, str) or not workspace_id.strip():
            raise ExportWorkspaceError("workspace_id must be a non-empty string")
        if not isinstance(workspace_name, str) or not workspace_name.strip():
            raise ExportWorkspaceError("workspace_name must be a non-empty string")
        if path is not None:
            cls._validate_path(path, home_dir)
        return cls(workspace_id=workspace_id, workspace_name=workspace_name, path=path)

    @staticmethod
    def _allowed_roots(home_dir: Optional[str]) -> List[str]:
        roots: List[str] = []
        home = home_dir if home_dir is not None else os.path.expanduser("~")
        roots.append(os.path.abspath(home))
        xdg_doc = os.environ.get("XDG_DOCUMENTS_DIR", "")
        if xdg_doc:
            roots.append(os.path.abspath(xdg_doc))
        default_docs = os.path.join(home, "Documents")
        roots.append(os.path.abspath(default_docs))
        return roots

    @classmethod
    def _validate_path(cls, path: str, home_dir: Optional[str]) -> None:
        if not isinstance(path, str) or not path.strip():
            raise ExportPathError("path must be a non-empty string when provided")
        if not os.path.isabs(path):
            raise ExportPathError(f"path must be absolute: {path!r}")
        if not path.lower().endswith(".md"):
            raise ExportPathError(f"path must end with .md: {path!r}")
        # Lexical normalization only: never resolve() untrusted symlinks here.
        # The writer must re-check confinement at write time.
        normalized = os.path.normpath(path)
        roots = cls._allowed_roots(home_dir)
        confined = any(
            normalized == root or normalized.startswith(root + os.sep)
            for root in roots
        )
        if not confined:
            raise ExportPathError(
                f"path must live under the user's home or Documents/XDG dirs: {path!r}"
            )
        # Reject explicit parent traversal segments even after normalization.
        segments = normalized.split(os.sep)
        if ".." in segments:
            raise ExportPathError(f"path traversal is not permitted: {path!r}")


def _longest_backtick_run(text: str) -> int:
    longest = 0
    current = 0
    for ch in text:
        if ch == "`":
            current += 1
            if current > longest:
                longest = current
        else:
            current = 0
    return longest


def _fence_for(text: str) -> str:
    return "`" * max(3, _longest_backtick_run(text) + 1)


def _iso_utc(exported_at: Any) -> str:
    if exported_at is None:
        raise ValueError("exported_at must be injected, never None")
    if hasattr(exported_at, "astimezone"):
        import datetime as _dt

        aware = exported_at if exported_at.tzinfo else exported_at.replace(
            tzinfo=_dt.timezone.utc
        )
        return aware.astimezone(_dt.timezone.utc).isoformat().replace("+00:00", "Z")
    return str(exported_at)


def _message_text(message: Dict[str, Any], fence: Optional[str]) -> str:
    body = message.get("text", "")
    body = redact(str(body))
    if message.get("kind") == "code" or fence is not None:
        f = fence or _fence_for(body)
        return f"{f}\n{body}\n{f}"
    return body


def render_markdown(
    messages: Sequence[Dict[str, Any]],
    context_notes: Sequence[Dict[str, Any]],
    interface_refs: Sequence[Dict[str, Any]],
    exported_at: Any,
    workspace_name: str = "Workspace",
) -> str:
    """Render a deterministic Markdown transcript. Pure function."""
    lines: List[str] = []
    lines.append(f"# {redact(str(workspace_name))}")
    lines.append("")
    lines.append(f"Exported: {_iso_utc(exported_at)}")
    lines.append("")

    if messages:
        lines.append("## Messages")
        lines.append("")
        for message in messages:
            speaker = str(message.get("speaker", "system"))
            if speaker not in ("user", "agent", "system"):
                speaker = "system"
            timestamp = message.get("timestamp")
            header = f"**{speaker}**"
            if timestamp is not None:
                header += f" · {redact(str(timestamp))}"
            lines.append(header)
            lines.append("")
            lines.append(_message_text(message, message.get("fence")))
            lines.append("")

    if context_notes:
        lines.append("## Remembered context")
        lines.append("")
        for note in context_notes:
            note_text = redact(str(note.get("text", "")))
            lines.append(f"- {note_text}")
        lines.append("")

    if interface_refs:
        lines.append("## Interfaces")
        lines.append("")
        for ref in interface_refs:
            name = redact(str(ref.get("name", "interface")))
            interface_id = str(ref.get("id", ""))
            revision = ref.get("revision")
            link = f"[{name}](interface:{interface_id})"
            if revision is not None:
                lines.append(f"- {link} (revision: {redact(str(revision))})")
            else:
                lines.append(f"- {link}")
        lines.append("")

    return "\n".join(lines).rstrip("\n") + "\n"


@dataclass(frozen=True)
class ExportResult:
    """Counts and byte length for a rendered export."""

    markdown: str
    message_count: int
    note_count: int
    interface_ref_count: int
    byte_length: int


def compose_export(
    workspace_id: str,
    workspace_name: str,
    messages: Sequence[Dict[str, Any]],
    context_notes: Sequence[Dict[str, Any]],
    interface_refs: Sequence[Dict[str, Any]],
    exported_at: Any,
    path: Optional[str] = None,
    home_dir: Optional[str] = None,
) -> ExportResult:
    """Validate, redact, render, and count. Pure top-level helper."""
    request = ExportRequest.create(
        workspace_id=workspace_id,
        workspace_name=workspace_name,
        path=path,
        home_dir=home_dir,
    )
    markdown = render_markdown(
        messages=messages,
        context_notes=context_notes,
        interface_refs=interface_refs,
        exported_at=exported_at,
        workspace_name=request.workspace_name,
    )
    return ExportResult(
        markdown=markdown,
        message_count=len(messages),
        note_count=len(context_notes),
        interface_ref_count=len(interface_refs),
        byte_length=len(markdown.encode("utf-8")),
    )
