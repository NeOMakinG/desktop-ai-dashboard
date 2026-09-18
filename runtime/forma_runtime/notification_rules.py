"""Pure in-memory notification rules engine for Forma's workspace.

Users describe when they want to be nudged: when enough activity of a
given kind happens in a scope within a time window. This module stores
those rules and evaluates injected event lists against them. Nothing
here performs I/O, reads a real clock, uses threads, or touches the
filesystem: scopes are opaque strings, dict copies go out, ordering is
deterministic - in the style of the existing attachments.py /
reading_list.py / clipboard_history.py / downloads.py modules.
"""

MIN_THRESHOLD = 1


def _resolve_now(clock, now):
    return clock() if now is None else now


class NotificationRules:
    def __init__(self, clock):
        if not callable(clock):
            raise TypeError("clock must be callable")
        self._clock = clock
        self._rules = {}
        self._seq = 0

    def _next_id(self):
        self._seq += 1
        return "nrule-%06d" % self._seq

    def add_rule(self, kind, scope="", threshold=MIN_THRESHOLD,
                 window_seconds=None, now=None):
        if not isinstance(kind, str) or not kind.strip():
            raise ValueError("kind must be a non-empty string")
        if not isinstance(scope, str):
            raise ValueError("scope must be a string")
        if not isinstance(threshold, int) or isinstance(threshold, bool):
            raise ValueError("threshold must be an integer")
        if threshold < MIN_THRESHOLD:
            raise ValueError("threshold must be at least one")
        if window_seconds is not None:
            if not isinstance(window_seconds, int) or isinstance(window_seconds, bool):
                raise ValueError("window_seconds must be an integer or None")
            if window_seconds < 1:
                raise ValueError("window_seconds must be positive")
        rule = {
            "id": self._next_id(),
            "kind": kind,
            "scope": scope,
            "threshold": threshold,
            "window_seconds": window_seconds,
            "enabled": True,
            "created_at": _resolve_now(self._clock, now),
            "removed": False,
        }
        self._rules[rule["id"]] = rule
        return dict(rule)

    def _get_live(self, rule_id):
        rule = self._rules.get(rule_id)
        if rule is None or rule["removed"]:
            raise KeyError(rule_id)
        return rule

    def enable(self, rule_id):
        rule = self._get_live(rule_id)
        if rule["enabled"]:
            return False
        rule["enabled"] = True
        return True

    def disable(self, rule_id):
        rule = self._get_live(rule_id)
        if not rule["enabled"]:
            return False
        rule["enabled"] = False
        return True

    def remove(self, rule_id):
        rule = self._rules.get(rule_id)
        if rule is None:
            raise KeyError(rule_id)
        if rule["removed"]:
            return False
        rule["removed"] = True
        return True

    def get(self, rule_id):
        return dict(self._get_live(rule_id))

    def _ordered(self, scope, include_removed):
        rows = [
            r for r in self._rules.values()
            if (include_removed or not r["removed"])
            and (scope is None or r["scope"] == scope)
        ]
        rows.sort(key=lambda r: (-r["created_at"], r["id"]))
        return [dict(r) for r in rows]

    def list_rules(self, scope=None, include_removed=False):
        if scope is not None and not isinstance(scope, str):
            raise ValueError("scope must be a string or None")
        return self._ordered(scope, include_removed)

    def evaluate(self, events, now):
        """Return one notification dict per matching rule, in rule-id order.

        An event matches a rule when it is a dict whose 'type' equals the
        rule kind and whose 'scope' matches (a rule with an empty scope
        matches any event; otherwise event scope must equal rule scope).
        Window semantics mirror the daily digest: with a window, only
        events with now - window_seconds < ts <= now count; the lower
        boundary is exclusive, the upper inclusive. None means all
        history. A rule fires when matches >= threshold, at most once
        per call, and evaluation is stateless and deterministic.
        """
        if not isinstance(now, (int, float)) or isinstance(now, bool):
            raise ValueError("now must be a number")
        fired = []
        live = [r for r in self._rules.values()
                if r["enabled"] and not r["removed"]]
        live.sort(key=lambda r: r["id"])
        for rule in live:
            matches = 0
            for event in events:
                if not isinstance(event, dict):
                    continue
                if event.get("type") != rule["kind"]:
                    continue
                if rule["scope"] != "" and event.get("scope", "") != rule["scope"]:
                    continue
                window = rule["window_seconds"]
                ts = event.get("ts", 0)
                if not isinstance(ts, (int, float)) or isinstance(ts, bool):
                    continue
                if window is not None and not (now - window < ts <= now):
                    continue
                matches += 1
            if matches >= rule["threshold"]:
                fired.append({
                    "rule_id": rule["id"],
                    "kind": rule["kind"],
                    "scope": rule["scope"],
                    "count": matches,
                    "window_seconds": rule["window_seconds"],
                    "fired_at": now,
                })
        return fired
