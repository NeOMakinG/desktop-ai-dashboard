"""Pure data-plane contracts for the Forma browser tab registry (backlog job 1).

This module is deliberately pure: no browser launch, no CDP, no I/O, no network,
no threads, and no clock reads. All time is passed in explicitly by the caller.
A future implementation (per ADR 0004) imports these validated state machines;
nothing here launches or attaches to anything.

Invariants enforced here:
  * tab ids are stable, unique, and never reused for a registry's lifetime;
  * closed tabs keep tombstone state and reject late operations;
  * profiles map only to a validated absolute user-data-dir path (cookies,
    tokens, and credentials are structurally impossible to store);
  * at most one profile is active, and switching is refused with a typed
    conflict listing offending open tab ids while tabs of another profile
    remain open;
  * a tab enters ``agent_driving`` only via a recorded grant reference, and
    ``refresh_automation_status(now)`` forcibly returns expired or revoked
    grants' tabs to ``idle`` so a polling UI can never show stale state;
  * ``to_redacted_summary()`` strips URL query strings and fragments, keeping
    only scheme, host, and path, so tokens never reach chat or UI echoes.
"""

from __future__ import annotations

import os
from dataclasses import dataclass, field
from typing import Dict, List, Optional
from urllib.parse import urlsplit, urlunsplit

IDLE = "idle"
AGENT_DRIVING = "agent_driving"
AUTOMATION_STATES = (IDLE, AGENT_DRIVING)


class BrowserTabError(Exception):
    """Base type for browser tab registry errors."""


class UnknownTabError(BrowserTabError):
    """Raised when a tab id has never existed in this registry."""


class TabClosedError(BrowserTabError):
    """Raised when an operation references a closed (tombstoned) tab."""


class InvalidProfileError(BrowserTabError):
    """Raised for unknown profiles or invalid user-data-dir values."""


class ProfileSwitchConflictError(BrowserTabError):
    """Raised when switching the active profile while other-profile tabs are open.

    ``offending_tab_ids`` lists the open tab ids blocking the switch, sorted
    for deterministic output.
    """

    def __init__(self, profile_id: str, offending_tab_ids: List[str]):
        self.profile_id = profile_id
        self.offending_tab_ids = sorted(offending_tab_ids)
        super().__init__(
            "cannot activate profile %r while tabs remain open: %s"
            % (profile_id, ", ".join(self.offending_tab_ids))
        )


class InvalidGrantError(BrowserTabError):
    """Raised when a grant reference is missing or malformed."""


def redact_url(url: str) -> str:
    """Return scheme://host/path for ``url``, dropping query and fragment."""
    parts = urlsplit(url)
    return urlunsplit((parts.scheme, parts.netloc, parts.path, "", ""))


@dataclass
class TabRecord:
    """One tab's data-plane state. Extensible without breaking the summary."""

    tab_id: str
    url: str
    profile_id: str
    automation_state: str = IDLE
    is_open: bool = True
    grant_id: Optional[str] = None
    grant_expires_at: Optional[float] = None


@dataclass
class ProfileRecord:
    """A per-account browser profile.

    Stores ONLY ``profile_id -> user_data_dir``. There are intentionally no
    fields for cookies, tokens, or credentials, so they cannot be stored here.
    """

    profile_id: str
    user_data_dir: str


class BrowserTabRegistry:
    """Registry of browser tabs with strict validation and tombstones."""

    def __init__(self) -> None:
        self._tabs: Dict[str, TabRecord] = {}
        self._next_tab_number = 1
        self._active_tab_id: Optional[str] = None

    # -- tab lifecycle ----------------------------------------------------

    def open_tab(self, url: str, profile_id: str) -> TabRecord:
        """Mint a new tab with a stable unique id, never reused afterward."""
        if not url:
            raise BrowserTabError("url must be non-empty")
        if not profile_id:
            raise BrowserTabError("profile_id must be non-empty")
        tab_id = "tab-%d" % self._next_tab_number
        self._next_tab_number += 1
        record = TabRecord(tab_id=tab_id, url=url, profile_id=profile_id)
        self._tabs[tab_id] = record
        if self._active_tab_id is None:
            self._active_tab_id = tab_id
        return record

    def _require_open(self, tab_id: str) -> TabRecord:
        record = self._tabs.get(tab_id)
        if record is None:
            raise UnknownTabError("unknown tab id: %r" % (tab_id,))
        if not record.is_open:
            raise TabClosedError("tab %r is closed" % (tab_id,))
        return record

    def close_tab(self, tab_id: str) -> TabRecord:
        """Close a tab, leaving a tombstone that rejects later operations."""
        record = self._require_open(tab_id)
        record.is_open = False
        record.automation_state = IDLE
        record.grant_id = None
        record.grant_expires_at = None
        if self._active_tab_id == tab_id:
            self._active_tab_id = None
        return record

    def activate_tab(self, tab_id: str) -> TabRecord:
        """Make an open tab the active tab; unknown/closed ids raise."""
        record = self._require_open(tab_id)
        self._active_tab_id = tab_id
        return record

    @property
    def active_tab_id(self) -> Optional[str]:
        return self._active_tab_id

    def get_tab(self, tab_id: str) -> TabRecord:
        """Return the record (open or tombstoned) for inspection."""
        record = self._tabs.get(tab_id)
        if record is None:
            raise UnknownTabError("unknown tab id: %r" % (tab_id,))
        return record

    def open_tab_ids(self, profile_id: Optional[str] = None) -> List[str]:
        """Sorted ids of open tabs, optionally filtered by profile."""
        return sorted(
            tid
            for tid, rec in self._tabs.items()
            if rec.is_open and (profile_id is None or rec.profile_id == profile_id)
        )

    # -- automation status -------------------------------------------------

    def set_agent_driving(
        self, tab_id: str, grant_id: str, expires_at: float
    ) -> TabRecord:
        """Enter agent_driving only with a recorded grant reference.

        Liveness (not-yet-expired, not revoked) is enforced by
        ``refresh_automation_status(now)`` and ``revoke_grant``; time is always
        caller-supplied.
        """
        record = self._require_open(tab_id)
        if not grant_id:
            raise InvalidGrantError("grant_id must be non-empty")
        if expires_at is None:
            raise InvalidGrantError("expires_at is required")
        record.automation_state = AGENT_DRIVING
        record.grant_id = grant_id
        record.grant_expires_at = float(expires_at)
        return record

    def set_idle(self, tab_id: str) -> TabRecord:
        """Manually return an open tab to idle."""
        record = self._require_open(tab_id)
        record.automation_state = IDLE
        record.grant_id = None
        record.grant_expires_at = None
        return record

    def revoke_grant(self, grant_id: str) -> List[str]:
        """Immediately force tabs using ``grant_id`` back to idle.

        Revocation lists are caller-supplied; this method applies one. Returns
        the sorted list of tab ids forced idle.
        """
        if not grant_id:
            raise InvalidGrantError("grant_id must be non-empty")
        forced = []
        for tid in sorted(self._tabs):
            rec = self._tabs[tid]
            if rec.is_open and rec.automation_state == AGENT_DRIVING and rec.grant_id == grant_id:
                rec.automation_state = IDLE
                rec.grant_id = None
                rec.grant_expires_at = None
                forced.append(tid)
        return forced

    def refresh_automation_status(self, now: float) -> List[str]:
        """Force expired or revoked grants' tabs to idle; return their ids.

        ``now`` is explicit; no clock is read here. Tabs whose grant has
        expired (``expires_at <= now``) or whose grant id was revoked are
        returned to idle so a polling UI can never show stale agent_driving.
        """
        forced = []
        for tid in sorted(self._tabs):
            rec = self._tabs[tid]
            if not rec.is_open or rec.automation_state != AGENT_DRIVING:
                continue
            expired = (
                rec.grant_id is None
                or rec.grant_expires_at is None
                or rec.grant_expires_at <= float(now)
            )
            if expired:
                rec.automation_state = IDLE
                rec.grant_id = None
                rec.grant_expires_at = None
                forced.append(tid)
        return forced

    # -- summary -----------------------------------------------------------

    def to_redacted_summary(self) -> Dict[str, object]:
        """Single UI-rendering source: redacted URLs, honest automation state."""
        tabs = []
        agent_driven = []
        for tid in sorted(self._tabs):
            rec = self._tabs[tid]
            if not rec.is_open:
                continue
            if rec.automation_state == AGENT_DRIVING:
                agent_driven.append(tid)
            tabs.append(
                {
                    "tab_id": tid,
                    "profile_id": rec.profile_id,
                    "automation_state": rec.automation_state,
                    "redacted_url": redact_url(rec.url),
                    "is_active": tid == self._active_tab_id,
                }
            )
        return {
            "active_tab_id": self._active_tab_id,
            "tabs": tabs,
            "agent_driven_tab_ids": agent_driven,
        }


class ProfileRegistry:
    """Per-account profiles; exactly one may be active at a time.

    Constructed with the :class:`BrowserTabRegistry` whose open tabs gate
    profile switching, so a switch is refused (typed conflict listing offending
    tab ids) while any open tab belongs to another profile.
    """

    def __init__(self, tab_registry: Optional[BrowserTabRegistry] = None) -> None:
        self._profiles: Dict[str, ProfileRecord] = {}
        self._active_profile_id: Optional[str] = None
        self._tab_registry = tab_registry

    def add_profile(self, profile_id: str, user_data_dir: str) -> ProfileRecord:
        """Register a profile mapped only to an absolute user-data-dir path."""
        if not profile_id:
            raise InvalidProfileError("profile_id must be non-empty")
        if not user_data_dir or not user_data_dir.strip():
            raise InvalidProfileError("user_data_dir must be non-empty")
        if not os.path.isabs(user_data_dir):
            raise InvalidProfileError(
                "user_data_dir must be absolute: %r" % (user_data_dir,)
            )
        if profile_id in self._profiles:
            raise InvalidProfileError("profile already registered: %r" % (profile_id,))
        record = ProfileRecord(profile_id=profile_id, user_data_dir=user_data_dir)
        self._profiles[profile_id] = record
        return record

    def get_profile(self, profile_id: str) -> ProfileRecord:
        record = self._profiles.get(profile_id)
        if record is None:
            raise InvalidProfileError("unknown profile id: %r" % (profile_id,))
        return record

    @property
    def active_profile_id(self) -> Optional[str]:
        return self._active_profile_id

    def set_active_profile(self, profile_id: str) -> str:
        """Activate a profile; refuse while other-profile tabs remain open."""
        self.get_profile(profile_id)  # raises InvalidProfileError when unknown
        if self._tab_registry is not None:
            offending = [
                tid
                for tid in self._tab_registry.open_tab_ids()
                if self._tab_registry.get_tab(tid).profile_id != profile_id
            ]
            if offending:
                raise ProfileSwitchConflictError(profile_id, offending)
        self._active_profile_id = profile_id
        return profile_id

    def profile_ids(self) -> List[str]:
        """Sorted registered profile ids (deterministic ordering)."""
        return sorted(self._profiles)
