"""Pure-Python contracts for executing Composio actions from Hermes.

Native-host pattern: the model emits a structured ActionRequest, the host
validates it against an explicit ActionGrant held in a GrantBook, executes it
through a ComposioTransport implementation host-side, and returns a sanitized
ActionReceipt.

This module performs no network calls, no subprocess execution, and no
credential handling. Real HTTP execution against the Composio API is a host
responsibility and must live outside this module, behind the abstract
ComposioTransport interface.

Grant validation error precedence (checked in this order):
1. UnknownConnectorGrant - no grant recorded for the connector
2. GrantRevoked       - the grant exists but is revoked
3. GrantExpired       - now >= expires_at (the boundary instant counts as expired)
4. ActionNotGranted   - the action is outside allowed_actions
"""

from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from datetime import datetime
from typing import Any, Dict, List, Optional


class ComposioActionError(Exception):
    """Base class for typed action validation errors."""


class UnknownConnectorGrant(ComposioActionError):
    """No explicit grant exists for the requested connector."""


class GrantRevoked(ComposioActionError):
    """The grant for the requested connector has been revoked."""


class GrantExpired(ComposioActionError):
    """The grant has expired; now >= expires_at counts as expired."""


class ActionNotGranted(ComposioActionError):
    """The requested action is outside the grant's allowed_actions."""


@dataclass
class ActionRequest:
    """Structured request emitted by the model for a Composio action."""

    request_id: str
    connector: str
    action: str
    arguments: Dict[str, Any]
    justification: str


@dataclass
class ActionGrant:
    """An explicit, expiring, revocable operator grant for a connector."""

    connector: str
    allowed_actions: set
    granted_at: datetime
    expires_at: datetime
    revoked: bool = False


@dataclass
class ActionReceipt:
    """Sanitized result of an action execution returned to the model."""

    VALID_STATUSES = frozenset({"succeeded", "failed", "denied"})

    request_id: str
    status: str
    sanitized_payload: Dict[str, Any] = field(default_factory=dict)

    def __post_init__(self) -> None:
        if self.status not in self.VALID_STATUSES:
            raise ValueError(
                "status must be one of %r, got %r"
                % (sorted(self.VALID_STATUSES), self.status)
            )


_SENSITIVE_SUBSTRINGS = ("token", "secret", "password", "key", "api_key")


def _is_sensitive_key(key: str) -> bool:
    lowered = key.lower()
    return any(sub in lowered for sub in _SENSITIVE_SUBSTRINGS)


def redact(payload: Any) -> Any:
    """Mask values whose keys contain sensitive substrings, recursively.

    Keys containing token, secret, password, key, or api_key (case-insensitive
    substring match) have their values replaced with "***REDACTED***". Nested
    dicts are recursed; non-matching keys pass through unchanged. This is
    fail-closed: benign keys containing a substring (e.g. "monkey") are also
    masked.
    """
    if isinstance(payload, dict):
        return {
            k: ("***REDACTED***" if _is_sensitive_key(k) else redact(v))
            for k, v in payload.items()
        }
    return payload


class GrantBook:
    """Host-owned store of explicit operator grants per connector."""

    def __init__(self) -> None:
        self._grants: Dict[str, ActionGrant] = {}

    def grant(self, grant: ActionGrant) -> None:
        """Record or replace an explicit grant for a connector."""
        self._grants[grant.connector] = grant

    def revoke(self, connector: str) -> Optional[ActionGrant]:
        """Revoke the grant for a connector, returning it if present."""
        grant = self._grants.get(connector)
        if grant is not None:
            grant.revoked = True
        return grant

    def validate(self, request: ActionRequest, now: datetime) -> ActionGrant:
        """Validate a request against the recorded grant.

        Raises, in precedence order: UnknownConnectorGrant, GrantRevoked,
        GrantExpired (now >= expires_at), ActionNotGranted.
        """
        grant = self._grants.get(request.connector)
        if grant is None:
            raise UnknownConnectorGrant(
                "no grant recorded for connector %r" % request.connector
            )
        if grant.revoked:
            raise GrantRevoked("grant for connector %r is revoked" % request.connector)
        if now >= grant.expires_at:
            raise GrantExpired(
                "grant for connector %r expired at %s (now=%s)"
                % (request.connector, grant.expires_at.isoformat(), now.isoformat())
            )
        if request.action not in grant.allowed_actions:
            raise ActionNotGranted(
                "action %r is not granted for connector %r"
                % (request.action, request.connector)
            )
        return grant


class ComposioTransport(ABC):
    """Abstract host-side executor for Composio actions.

    Real HTTP execution against the Composio API is a host responsibility and
    must never be implemented in this module.
    """

    @abstractmethod
    def execute(self, request: ActionRequest) -> ActionReceipt:
        """Execute a validated request and return a sanitized receipt."""
        raise NotImplementedError


class StubTransport(ComposioTransport):
    """Test double: records requests in order and returns canned receipts."""

    def __init__(
        self, receipts: Optional[List[ActionReceipt]] = None
    ) -> None:
        self.recorded_requests: List[ActionRequest] = []
        self._receipts = list(receipts) if receipts else []

    def execute(self, request: ActionRequest) -> ActionReceipt:
        self.recorded_requests.append(request)
        if self._receipts:
            receipt = self._receipts.pop(0)
        else:
            receipt = ActionReceipt(
                request_id=request.request_id, status="succeeded"
            )
        if receipt.request_id != request.request_id:
            receipt.request_id = request.request_id
        return receipt
