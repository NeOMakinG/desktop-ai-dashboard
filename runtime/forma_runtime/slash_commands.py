"""Pure data-plane module for the chat slash command palette (F09).

Registry + parse + suggest + route for workspace slash commands. This module
performs NO IO: no filesystem, clock, environment, or network access. Imports
are limited to dataclasses, typing, and json. route() emits plain-JSON action
descriptors consumable by existing runtime capabilities:

- summarize  -> thread summarizer
- export     -> workspace Markdown export (compose_export/render_markdown semantics)
- pin        -> workspace pin (src/domain/workspace-pin.ts semantics)
- dashboard  -> open dashboard
- clear-context -> clear remembered context
- help       -> show help
"""

from dataclasses import dataclass, field
from typing import Any, Dict, List, Optional
import json


@dataclass(frozen=True)
class ArgumentSpec:
    """Spec for a command's free-text argument."""

    required: bool = False
    free_text: bool = True


@dataclass(frozen=True)
class CommandEntry:
    """One registry entry for the slash command palette."""

    name: str
    description: str
    argument_spec: ArgumentSpec = field(default_factory=ArgumentSpec)
    action: str = ""
    target: str = "workspace"


@dataclass(frozen=True)
class ParsedCommand:
    """Result of a successful parse."""

    name: str
    args: Dict[str, Any]
    raw: str


@dataclass(frozen=True)
class ParseError(Exception):
    """Raised when text is not a command or names an unknown command."""

    kind: str  # "not_a_command" | "unknown_command"
    message: str
    suggestions: List[str] = field(default_factory=list)


# Registry kept in deterministic alphabetical order by name so that both
# registry order and suggest() order are alphabetical.
COMMANDS: List[CommandEntry] = [
    CommandEntry(
        name="clear-context",
        description="Clear remembered context for this workspace",
        argument_spec=ArgumentSpec(required=False, free_text=False),
        action="clear_context",
        target="workspace",
    ),
    CommandEntry(
        name="dashboard",
        description="Open the dashboard view",
        argument_spec=ArgumentSpec(required=False, free_text=False),
        action="open_dashboard",
        target="app",
    ),
    CommandEntry(
        name="export",
        description="Export this workspace to a Markdown file",
        argument_spec=ArgumentSpec(required=False, free_text=True),
        action="workspace_export",
        target="workspace",
    ),
    CommandEntry(
        name="help",
        description="Show help for available slash commands",
        argument_spec=ArgumentSpec(required=False, free_text=True),
        action="show_help",
        target="app",
    ),
    CommandEntry(
        name="pin",
        description="Pin this workspace to the sidebar",
        argument_spec=ArgumentSpec(required=False, free_text=False),
        action="workspace_pin",
        target="workspace",
    ),
    CommandEntry(
        name="summarize",
        description="Summarize the current thread with the summarizer",
        argument_spec=ArgumentSpec(required=False, free_text=True),
        action="summarize_thread",
        target="thread",
    ),
]

_COMMANDS_BY_NAME: Dict[str, CommandEntry] = {entry.name: entry for entry in COMMANDS}


def suggest(prefix: str) -> List[CommandEntry]:
    """Return registry entries whose name starts with prefix, alphabetical."""

    normalized = (prefix or "").lstrip("/").lower()
    return [
        entry
        for entry in COMMANDS
        if entry.name.startswith(normalized)
    ]


def _near_matches(name: str) -> List[str]:
    """Nearest valid names via prefix matching; exact near-miss included."""

    lowered = name.lower()
    matches = [entry.name for entry in suggest(lowered)]
    return sorted(matches)


def parse(text: str) -> ParsedCommand:
    """Parse chat text into a ParsedCommand or raise ParseError.

    Inputs not starting with '/' are rejected as not-a-command. Unknown or
    misspelled commands raise a ParseError listing nearest valid names via
    prefix matching.
    """

    if not isinstance(text, str) or not text.startswith("/"):
        raise ParseError(
            kind="not_a_command",
            message="Input does not start with '/' and is not a command",
            suggestions=[],
        )

    body = text[1:].strip()
    if not body:
        raise ParseError(
            kind="unknown_command",
            message="No command name given after '/'",
            suggestions=[entry.name for entry in COMMANDS],
        )

    parts = body.split(None, 1)
    name = parts[0].lower()
    arg_text = parts[1].strip() if len(parts) > 1 else ""

    entry = _COMMANDS_BY_NAME.get(name)
    if entry is None:
        near = _near_matches(name)
        if near:
            message = "Unknown command '/{}'; did you mean: {}".format(
                name, ", ".join("/" + candidate for candidate in near)
            )
        else:
            message = "Unknown command '/{}'".format(name)
        raise ParseError(
            kind="unknown_command",
            message=message,
            suggestions=near,
        )

    args: Dict[str, Any] = {}
    if entry.argument_spec.free_text:
        if entry.argument_spec.required and not arg_text:
            raise ParseError(
                kind="unknown_command",
                message="Command '/{}' requires an argument".format(name),
                suggestions=[],
            )
        args["text"] = arg_text

    return ParsedCommand(name=entry.name, args=args, raw=text)


def route(parsed: ParsedCommand) -> Dict[str, Any]:
    """Map a ParsedCommand to a plain-JSON action descriptor.

    Returns {"action": str, "target": str, "args": dict}. Performs NO IO.
    """

    entry = _COMMANDS_BY_NAME.get(parsed.name)
    if entry is None:
        raise ValueError("Unknown command name: {!r}".format(parsed.name))

    descriptor = {
        "action": entry.action,
        "target": entry.target,
        "args": dict(parsed.args),
    }
    # Round-trip through json to guarantee the descriptor is plain JSON
    # (no dataclasses, sets, or other non-JSON types sneak through).
    return json.loads(json.dumps(descriptor))


__all__ = [
    "ArgumentSpec",
    "CommandEntry",
    "ParsedCommand",
    "ParseError",
    "COMMANDS",
    "parse",
    "suggest",
    "route",
]
