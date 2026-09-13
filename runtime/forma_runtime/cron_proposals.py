"""Pure contract for chat-originated cron scheduling proposals.

The model proposes genuine cron schedules as structured proposals; the host
explicitly approves or denies each one before anything is scheduled. This
module performs no OS scheduling, execution, network, subprocess, or thread
activity.
"""

from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Dict, List, Optional
import re

# Per-field bounds: minute, hour, day-of-month, month, day-of-week
_FIELD_BOUNDS = ((0, 59), (0, 23), (1, 31), (1, 12), (0, 7))

_ELEMENT_RE = re.compile(r"^(\d+)(?:-(\d+))?(?:/(\d+))?$")

_STATUSES = ("proposed", "approved", "denied")


def _validate_cron_field(text: str, index: int, name: str) -> None:
    low, high = _FIELD_BOUNDS[index]
    for element in text.split(","):
        if element == "*":
            continue
        if element.startswith("*/"):
            step_part = element[2:]
            if not step_part.isdigit() or int(step_part) < 1:
                raise ValueError(
                    f"invalid step in cron field {name}: {element!r}"
                )
            continue
        match = _ELEMENT_RE.match(element)
        if match is None:
            raise ValueError(
                f"invalid token {element!r} in cron field {name}"
            )
        start = int(match.group(1))
        end = int(match.group(2)) if match.group(2) is not None else start
        step = int(match.group(3)) if match.group(3) is not None else None
        if step is not None and step < 1:
            raise ValueError(f"invalid step in cron field {name}: {element!r}")
        if not (low <= start <= high) or not (low <= end <= high):
            raise ValueError(
                f"value out of range in cron field {name}: {element!r}"
            )
        if end < start:
            raise ValueError(
                f"inverted range in cron field {name}: {element!r}"
            )


def validate_cron_expression(expression: str) -> None:
    """Validate a strict five-field cron expression or raise ValueError."""
    if not isinstance(expression, str):
        raise ValueError("cron expression must be a string")
    stripped = expression.strip()
    if stripped.startswith("@"):
        raise ValueError(f"@macros are not allowed: {expression!r}")
    fields_ = stripped.split()
    if len(fields_) != 5:
        raise ValueError(
            "cron expression must have exactly 5 fields, "
            f"got {len(fields_)}: {expression!r}"
        )
    names = ("minute", "hour", "day-of-month", "month", "day-of-week")
    for index, (name, value) in enumerate(zip(names, fields_)):
        _validate_cron_field(value, index, name)


@dataclass
class CronProposal:
    """A model-proposed cron schedule awaiting an explicit host decision."""

    id: str
    cron_expression: str
    action: Dict
    rationale: str
    created_at: datetime = field(
        default_factory=lambda: datetime.now(timezone.utc)
    )
    status: str = "proposed"

    def __post_init__(self) -> None:
        validate_cron_expression(self.cron_expression)
        if not isinstance(self.action, dict):
            raise ValueError("action must be a plain dict")
        # Shallow defensive copy: action is a plain descriptor dict.
        self.action = dict(self.action)
        if self.status not in _STATUSES:
            raise ValueError(f"status must be one of {_STATUSES}")


@dataclass
class _Decision:
    approved: bool
    decided_at: datetime


class ApprovalLedger:
    """Records explicit host approve/deny decisions; never auto-approves."""

    def __init__(self) -> None:
        self._proposals: Dict[str, CronProposal] = {}
        self._decisions: Dict[str, _Decision] = {}

    def add(self, proposal: CronProposal) -> None:
        if proposal.id in self._proposals:
            raise ValueError(f"proposal id already present: {proposal.id!r}")
        self._proposals[proposal.id] = proposal

    def decide(self, proposal_id: str, approved: bool) -> None:
        """Record an explicit host decision for a proposal."""
        proposal = self._proposals.get(proposal_id)
        if proposal is None:
            raise ValueError(f"unknown proposal: {proposal_id!r}")
        if proposal_id in self._decisions:
            raise ValueError(
                f"proposal already decided: {proposal_id!r}"
            )
        self._decisions[proposal_id] = _Decision(
            approved=bool(approved), decided_at=datetime.now(timezone.utc)
        )
        proposal.status = "approved" if approved else "denied"

    def decision(self, proposal_id: str) -> Optional[_Decision]:
        return self._decisions.get(proposal_id)

    @property
    def proposals(self) -> List[CronProposal]:
        return list(self._proposals.values())


def approved_proposals(ledger: ApprovalLedger) -> List[CronProposal]:
    """Return only approved proposals. Pure: no scheduling or execution."""
    return [p for p in ledger.proposals if p.status == "approved"]
