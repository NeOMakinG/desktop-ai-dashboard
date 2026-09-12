"""Transactional product state. A single SQLite writer owns durable fences."""
from __future__ import annotations

import hashlib
import hmac
import fcntl
import os
import json
import secrets
import sqlite3
import threading
from contextlib import contextmanager
from datetime import datetime, timedelta
from pathlib import Path

from .contracts import *


class Store:
    def __init__(self, path, library_id, device_id, models, ready=lambda: False):
        require((library_id is None) == (device_id is None), "Incomplete database identity")
        if library_id is not None: uid(library_id); uid(device_id)
        self.library_id, self.device_id, self.models, self.ready = library_id, device_id, models, ready
        self.lock = threading.RLock()
        self.owner_fd = os.open(str(path) + ".lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
        try:
            fcntl.flock(self.owner_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except OSError as exc:
            os.close(self.owner_fd)
            raise Fault("database_in_use", "Another runtime owns this database", 503) from exc
        self.db = sqlite3.connect(str(path), check_same_thread=False, isolation_level=None)
        self.db.row_factory = sqlite3.Row
        self.db.executescript("""
            PRAGMA journal_mode=DELETE;
            PRAGMA synchronous=FULL;
            PRAGMA foreign_keys=ON;
            CREATE TABLE IF NOT EXISTS objects(kind TEXT NOT NULL,id TEXT NOT NULL,body TEXT NOT NULL,
              PRIMARY KEY(kind,id));
            CREATE TABLE IF NOT EXISTS events(run_id TEXT NOT NULL,seq INTEGER NOT NULL,body TEXT NOT NULL,
              PRIMARY KEY(run_id,seq));
            CREATE TABLE IF NOT EXISTS mutations(path TEXT NOT NULL,key TEXT NOT NULL,hash TEXT NOT NULL,
              status INTEGER NOT NULL,body TEXT NOT NULL,PRIMARY KEY(path,key));
            CREATE TABLE IF NOT EXISTS occurrences(schedule_id TEXT NOT NULL,slot TEXT NOT NULL,run_id TEXT,
              outcome TEXT NOT NULL,PRIMARY KEY(schedule_id,slot));
        """)
        with self.transaction():
            binding = self.get("meta", "identity", optional=True)
            expected = {"libraryId": library_id, "deviceId": device_id} if library_id is not None else (
                binding or {"libraryId": new_id(), "deviceId": new_id()})
            obj(expected, ("libraryId", "deviceId")); uid(expected["libraryId"]); uid(expected["deviceId"])
            require(binding is None or binding == expected, "Database identity mismatch", "identity_mismatch", 503)
            self.library_id, self.device_id = expected["libraryId"], expected["deviceId"]
            self.put("meta", "identity", expected)

    @contextmanager
    def transaction(self):
        with self.lock:
            self.db.execute("BEGIN IMMEDIATE")
            try:
                yield
                self.db.execute("COMMIT")
            except BaseException:
                self.db.execute("ROLLBACK")
                raise

    def close(self):
        with self.lock:
            self.db.close()
            fcntl.flock(self.owner_fd, fcntl.LOCK_UN); os.close(self.owner_fd)

    def get(self, kind, key, optional=False, tombstone=False):
        row = self.db.execute("SELECT body FROM objects WHERE kind=? AND id=?", (kind, key)).fetchone()
        if row is None:
            if optional: return None
            raise Fault("not_found", "Object not found", 404)
        result = json.loads(row["body"])
        require(tombstone or not result.get("_deletedAt"), "Object deleted", "tombstoned", 410)
        return result

    def put(self, kind, key, body):
        self.db.execute("INSERT INTO objects VALUES(?,?,?) ON CONFLICT(kind,id) DO UPDATE SET body=excluded.body",
                        (kind, key, canonical(body)))

    def all(self, kind, tombstones=False):
        values = [json.loads(r[0]) for r in self.db.execute("SELECT body FROM objects WHERE kind=? ORDER BY rowid", (kind,))]
        return [x for x in values if tombstones or not x.get("_deletedAt")]

    @staticmethod
    def public(value):
        return {key: item for key, item in value.items() if not key.startswith("_")}

    def event(self, run, kind, payload):
        run["lastSeq"] += 1
        event = {"runId": run["id"], "seq": run["lastSeq"], "at": stamp(), "type": kind, "payload": payload}
        self.db.execute("INSERT INTO events VALUES(?,?,?)", (run["id"], run["lastSeq"], canonical(event)))
        self.put("run", run["id"], run)

    def state(self, run, state, reason=None):
        run["state"] = state
        if reason: run["reason"] = reason
        else: run.pop("reason", None)
        if state in TERMINAL: run["finishedAt"] = stamp()
        self.event(run, "run.state", {"state": state, **({"reason": reason} if reason else {})})

    def model(self, model_id):
        require(any(x["id"] == model_id and x.get("available", True) for x in self.models),
                "Selected model unavailable", "model_unavailable", 422)

    def workspace(self, wid, imported=None, admission=False):
        record = self.get("workspace", wid, optional=True)
        if record is None:
            record = {"id": wid, "history": [], "createdAt": stamp(), "_admitted": False}
        require(not (record.get("_admitted") and imported), "Workspace history already initialized", "history_already_initialized", 409)
        if admission and not record.get("_admitted"):
            record["history"] = imported or []; record["_admitted"] = True
        self.put("workspace", wid, record)
        return record

    def check_grants(self, wid, model_id, grant_refs):
        grants = []
        for ref in grant_refs:
            grant = self.get("grant", ref["id"])
            require(grant["workspaceId"] == wid and grant["modelId"] == model_id
                    and grant["deviceId"] == self.device_id and grant["generation"] == ref["generation"]
                    and not grant["revoked"] and instant(grant["expiresAt"]) > datetime.now(UTC)
                    and grant["dataMode"] == "synthetic", "Grant not current for this run", "grant_denied", 403)
            grants.append(grant)
        return grants

    def fence(self, run):
        require(run["state"] not in TERMINAL and run["state"] != "cancelling", "Run no longer active", "run_inactive", 409)
        self.get("workspace", run["workspaceId"])
        self.model(run["modelId"])
        require(instant(run["_deadline"]) > datetime.now(UTC), "Run deadline exceeded", "deadline_exceeded", 410)
        self.check_grants(run["workspaceId"], run["modelId"], run["_input"]["grantRefs"])
        for iid in run["_input"]["interfaceIds"]: self.get("interface", iid)
        if "scheduleId" in run:
            schedule = self.get("schedule", run["scheduleId"])
            require(schedule["state"] in ("enabled", "ended") and schedule["version"] == run["_scheduleVersion"]
                    and instant(schedule["endAt"]) > datetime.now(UTC), "Schedule authorization changed", "schedule_fenced", 409)
        return run

    def create_run(self, value, rid, schedule=None):
        run_input(value); self.model(value["modelId"])
        require(self.ready(), "Dedicated Hermes worker unavailable", "runtime_unavailable", 503)
        require(self.get("cancellation", rid, optional=True) is None, "Run cancelled before admission", "run_cancelled_before_admission", 410)
        require(self.get("run", rid, optional=True) is None, "Run ID already admitted", "run_exists", 409)
        wid = value["workspaceId"]
        ws = self.workspace(wid, value.get("importHistory"), admission=schedule is None)
        grants = self.check_grants(wid, value["modelId"], value["grantRefs"])
        for iid in value["interfaceIds"]: self.get("interface", iid)
        active = [x for x in self.all("run") if x["state"] not in TERMINAL]
        require(len(active) < 8, "Runtime queue full", "capacity_exceeded", 429)
        connections = {g["connectionId"] for g in grants}
        require(not any(x["workspaceId"] == wid or connections.intersection(x["_connections"]) for x in active),
                "Workspace or connection already has active work", "run_active", 409)
        now = datetime.now(UTC)
        run = {"id": rid, "workspaceId": wid, "origin": "schedule" if schedule else "operator",
               "state": "queued", "modelId": value["modelId"], "hermesRevision": REVISION,
               "budgets": value["budgets"], "createdAt": stamp(now), "lastSeq": 0,
               "_deadline": stamp(now + timedelta(seconds=value["budgets"]["maxDurationSeconds"])),
               "_input": value, "_history": [] if schedule else runtime_history(ws["history"]), "_connections": sorted(connections), "_toolCalls": 0}
        if schedule:
            run.update(scheduleId=schedule["id"], _scheduleVersion=schedule["version"],
                       _targetRevision=self.get("interface", schedule["interfaceId"])["revision"])
        self.state(run, "queued")
        return self.public(run)

    def post(self, path, key, body):
        uid(key); digest = hashlib.sha256(canonical(body).encode()).hexdigest()
        with self.transaction():
            previous = self.db.execute("SELECT * FROM mutations WHERE path=? AND key=?", (path, key)).fetchone()
            if previous:
                require(hmac.compare_digest(digest, previous["hash"]), "Idempotency key reused with another body",
                        "idempotency_conflict", 409)
                return previous["status"], json.loads(previous["body"])
            status, result = self._post(path.strip("/").split("/"), key, body)
            self.db.execute("INSERT INTO mutations VALUES(?,?,?,?,?)", (path, key, digest, status, canonical(result)))
            return status, result

    def _post(self, p, key, body):
        require(p and p[0] == "v1", "Route not found", "not_found", 404)
        if p == ["v1", "runs"]: return 202, self.create_run(body, key)
        if p == ["v1", "device-heartbeat"]:
            obj(body); value = {"deviceId": self.device_id, "onlineUntil": stamp(datetime.now(UTC) + timedelta(seconds=45))}
            self.put("device", self.device_id, value); return 200, value
        if len(p) == 4 and p[1] == "workspaces" and p[3] == "delete":
            obj(body); uid(p[2]); ws = self.get("workspace", p[2], optional=True, tombstone=True) or {"id": p[2]}
            ws.update(_deletedAt=stamp(), history=[]); self.put("workspace", p[2], ws)
            self.invalidate(lambda r: r["workspaceId"] == p[2], "workspace_deleted")
            for s in self.all("schedule"):
                if s["workspaceId"] == p[2]: self.pause(s, "workspace_deleted")
            return 200, {"id": p[2], "deletedAt": ws["_deletedAt"]}
        if p == ["v1", "grants"]:
            obj(body, ("workspaceId", "connectionId", "deviceId", "operations", "modelId", "expiresAt", "dataMode"))
            for k in ("workspaceId", "connectionId", "deviceId"): uid(body[k])
            require(body["deviceId"] == self.device_id and body["dataMode"] == "synthetic", "Live grants unavailable", "grant_denied", 403)
            operations = array(body["operations"], 2)
            require(bool(operations) and all(x in GOOGLE_TOOLS for x in operations) and len(set(operations)) == len(operations), "Invalid grant operations")
            require(instant(body["expiresAt"]) > datetime.now(UTC), "Grant already expired")
            self.model(body["modelId"]); self.workspace(body["workspaceId"])
            grant = dict(body, id=new_id(), generation=1, revoked=False)
            self.put("grant", grant["id"], grant); return 201, grant
        if len(p) == 4 and p[1] == "grants" and p[3] == "revoke":
            obj(body, ("generation",)); integer(body["generation"], 1)
            grant = self.get("grant", p[2]); require(body["generation"] == grant["generation"], "Grant changed", "revision_conflict", 409)
            grant.update(revoked=True, generation=grant["generation"] + 1); self.put("grant", p[2], grant)
            self.invalidate(lambda r: any(g["id"] == p[2] for g in r["_input"]["grantRefs"]), "grant_revoked")
            for s in self.all("schedule"):
                if any(g["id"] == p[2] for g in s["grantRefs"]): self.pause(s, "grant_revoked")
            for tr in self.all("tool"):
                if tr["grantRef"]["id"] == p[2]:
                    tr.pop("_result", None); tr.pop("_claim", None); tr["state"] = "invalidated"; self.put("tool", tr["id"], tr)
            return 200, grant
        if len(p) == 4 and p[1] == "runs":
            uid(p[2])
            if p[3] == "cancel":
                obj(body)
                run = self.get("run", p[2], optional=True)
                if run is None:
                    receipt = {"id": p[2], "state": "cancelled", "admitted": False}
                    self.put("cancellation", p[2], receipt)
                    return 202, receipt
                if run["state"] not in TERMINAL:
                    self.state(run, "cancelled" if run["state"] == "queued" else "cancelling", "operator_cancelled")
                    self.invalidate_proposals(run["id"])
                return 202, {"id": run["id"], "state": run["state"], "admitted": True, "run": self.public(run)}
            run = self.get("run", p[2])
            if p[3] in ("tool-claim", "tool-result"):
                return self.tool_response(run, p[3], body)
        if p == ["v1", "interfaces", "propose"]: return 201, self.propose(body)
        if p == ["v1", "interfaces", "publish"]:
            obj(body, ("proposalId", "expectedRevision")); uid(body["proposalId"]); integer(body["expectedRevision"])
            return 201, self.publish(body["proposalId"], body["expectedRevision"])
        if len(p) == 4 and p[1] == "interfaces":
            interface = self.get("interface", p[2]); action = p[3]
            fields = {"rename": ("expectedRevision", "title"), "delete": ("expectedRevision",),
                      "rollback": ("expectedRevision", "targetRevision")}
            require(action in fields, "Route not found", "not_found", 404); obj(body, fields[action])
            integer(body["expectedRevision"], 1)
            require(interface["revision"] == body["expectedRevision"], "Interface changed", "revision_conflict", 409)
            if action == "delete":
                interface["_deletedAt"] = stamp(); self.put("interface", p[2], interface)
                self.invalidate(lambda r: p[2] in r["_input"]["interfaceIds"], "interface_deleted")
                for s in self.all("schedule"):
                    if s["interfaceId"] == p[2]: self.pause(s, "interface_deleted")
                for proposal in self.all("proposal"):
                    if proposal.get("interfaceId") == p[2]: proposal["state"] = "invalidated"; self.put("proposal", proposal["id"], proposal)
                return 200, {"id": p[2], "deletedAt": interface["_deletedAt"]}
            if action == "rename":
                text(body["title"], 160, 1); title, spec = body["title"], interface["spec"]
            else:
                integer(body["targetRevision"], 1); old = self.get("revision", f'{p[2]}:{body["targetRevision"]}')
                title, spec = old["title"], old["spec"]
            return (200 if action == "rename" else 201), self.write_interface(interface, title, spec, interface["provenance"])
        if p == ["v1", "schedules"]: return 201, self.create_schedule(body)
        if len(p) == 4 and p[1] == "schedules":
            schedule = self.get("schedule", p[2]); action = p[3]
            require(action in ("enable", "pause", "delete"), "Route not found", "not_found", 404)
            obj(body, ("expectedVersion", "consent") if action == "enable" else ("expectedVersion",))
            integer(body["expectedVersion"], 1)
            require(schedule["version"] == body["expectedVersion"], "Schedule changed", "revision_conflict", 409)
            if action == "enable":
                consent = obj(body["consent"], ("scheduleVersion", "modelId", "grantRefs"))
                require(consent == {"scheduleVersion": schedule["version"], "modelId": schedule["modelId"], "grantRefs": schedule["grantRefs"]},
                        "Consent does not match schedule", "consent_mismatch", 403)
                require(self.ready(), "Hermes unavailable", "runtime_unavailable", 503)
                self.get("workspace", schedule["workspaceId"]); self.get("interface", schedule["interfaceId"])
                self.check_grants(schedule["workspaceId"], schedule["modelId"], schedule["grantRefs"])
                self.model(schedule["modelId"])
                require(schedule["runsStarted"] < schedule["maxRuns"] and instant(schedule["endAt"]) > datetime.now(UTC),
                        "Schedule bounds exhausted", "schedule_ended", 409)
                due = Cron(schedule["cron"]).next(datetime.now(UTC), instant(schedule["endAt"]))
                require(due is not None, "No future occurrence within schedule bounds")
                schedule.update(state="enabled", version=schedule["version"] + 1, updatedAt=stamp(), nextRunAt=due)
                schedule.pop("reason", None); self.put("schedule", schedule["id"], schedule)
            else:
                self.pause(schedule, "operator_paused" if action == "pause" else "schedule_deleted")
                if action == "delete":
                    schedule["_deletedAt"] = stamp(); self.put("schedule", schedule["id"], schedule)
                    return 200, {"id": schedule["id"], "deletedAt": schedule["_deletedAt"]}
            return 200, self.public(schedule)
        raise Fault("not_found", "Route not found", 404)

    @staticmethod
    def page(items, limit, cursor):
        selected = []
        for item in items[cursor:cursor + limit]:
            if len(canonical({"items": selected + [item]}).encode()) > 380000: break
            selected.append(item)
        next_offset = cursor + len(selected); more = next_offset < len(items)
        return {"items": selected, "hasMore": more, **({"nextCursor": str(next_offset)} if more else {})}

    def get_route(self, path, query):
        p = path.strip("/").split("/")
        require(p and p[0] == "v1", "Route not found", "not_found", 404)
        try:
            poll = len(p) == 3 and p[1] == "runs"
            limit = integer(int(query.get("limit", "200" if poll else "20")), 1, 200 if poll else 50)
            after = integer(int(query.get("after", "0"))); cursor = integer(int(query.get("cursor", "0")))
        except ValueError as exc: raise Fault("invalid_request", "Invalid cursor or limit") from exc
        with self.lock:
            if poll:
                require(self.get("cancellation", p[2], optional=True) is None, "Run cancelled before admission", "run_cancelled_before_admission", 410)
                run = self.get("run", p[2]); candidates = [json.loads(x[0]) for x in self.db.execute(
                    "SELECT body FROM events WHERE run_id=? AND seq>? ORDER BY seq LIMIT ?", (p[2], after, limit + 1))]
                requests = [self.public(t) for t in self.all("tool") if t["runId"] == p[2]
                            and t["state"] in ("pending", "claimed") and t["deviceId"] == self.device_id
                            and run["state"] not in TERMINAL and run["state"] != "cancelling"
                            and instant(t["expiresAt"]) > datetime.now(UTC)]
                result = {"run": self.public(run), "events": [], "nextAfter": after, "hasMore": False, "toolRequests": requests}
                for event in candidates[:limit]:
                    if len(canonical(result).encode()) + len(canonical(event).encode()) > 380000: break
                    result["events"].append(event); result["nextAfter"] = event["seq"]
                result["hasMore"] = len(candidates) > len(result["events"])
                return result
            if p == ["v1", "interfaces", "proposals"]: kind = "proposal"
            elif len(p) == 4 and p[1:3] == ["interfaces", "proposals"]: return self.public(self.get("proposal", p[3]))
            elif len(p) == 2 and p[1] in ("runs", "interfaces", "schedules"):
                kind = {"runs": "run", "interfaces": "interface", "schedules": "schedule"}[p[1]]
            elif len(p) == 3 and p[1] in ("interfaces", "schedules"):
                return self.public(self.get("interface" if p[1] == "interfaces" else "schedule", p[2]))
            elif len(p) in (4, 5) and p[1] == "interfaces" and p[3] == "revisions":
                self.get("interface", p[2])
                if len(p) == 5:
                    revision = integer(int(p[4]), 1)
                    return self.public(self.get("revision", f'{p[2]}:{revision}'))
                items = [{k: v for k, v in x.items() if k != "spec"} for x in reversed(self.all("revision")) if x["interfaceId"] == p[2]]
                return self.page(items, limit, cursor)
            else: raise Fault("not_found", "Route not found", 404)
            wid = uid(query.get("workspaceId"))
            self.get("workspace", wid, optional=True)
            items = [{k: v for k, v in self.public(x).items() if k not in ("spec", "prompt", "finalMessage")}
                     for x in reversed(self.all(kind)) if kind == "interface" or x.get("workspaceId") == wid]
            return self.page(items, limit, cursor)

    def propose(self, body, source_run=None):
        proposal_input(body); self.workspace(body["workspaceId"])
        if source_run:
            self.fence(source_run)
            require(body["workspaceId"] == source_run["workspaceId"], "Wrong workspace", "unauthorized_scope", 403)
        if "interfaceId" in body:
            target = self.get("interface", body["interfaceId"])
            require(target["revision"] == body["expectedRevision"], "Interface changed", "revision_conflict", 409)
        if source_run and "scheduleId" in source_run:
            schedule = self.get("schedule", source_run["scheduleId"])
            require(body.get("interfaceId") == schedule["interfaceId"] and body["expectedRevision"] == source_run["_targetRevision"],
                    "Scheduled write outside target", "unauthorized_scope", 403)
        proposal = dict(body, id=new_id(), state="pending", createdAt=stamp())
        if source_run: proposal["sourceRunId"] = source_run["id"]
        self.put("proposal", proposal["id"], proposal)
        if source_run:
            self.event(source_run, "interface.proposed", {k: proposal[k] for k in ("interfaceId", "expectedRevision") if k in proposal} | {"proposalId": proposal["id"]})
        return proposal

    def write_interface(self, current, title, spec, provenance):
        now = stamp(); iid = current["id"] if current else new_id(); revision = current["revision"] + 1 if current else 1
        value = {"id": iid, "libraryId": self.library_id, "revision": revision, "title": title, "spec": spec,
                 "createdAt": current["createdAt"] if current else now, "updatedAt": now, "provenance": provenance}
        historical = {"interfaceId": iid, "revision": revision, "title": title, "spec": spec, "createdAt": now}
        if current: historical["parentRevision"] = current["revision"]
        self.put("revision", f"{iid}:{revision}", historical); self.put("interface", iid, value)
        return value

    def publish(self, pid, expected, scheduled_run=None):
        proposal = self.get("proposal", pid)
        require(proposal["state"] == "pending", "Proposal no longer pending", "proposal_inactive", 409)
        require(proposal["expectedRevision"] == expected, "Proposal revision mismatch", "revision_conflict", 409)
        self.get("workspace", proposal["workspaceId"])
        source = None
        if "sourceRunId" in proposal:
            source = self.get("run", proposal["sourceRunId"])
            if scheduled_run: self.fence(source)
            else:
                require(source["state"] == "succeeded", "Source run has not succeeded", "run_inactive", 409)
                self.check_grants(source["workspaceId"], source["modelId"], source["_input"]["grantRefs"])
                if "scheduleId" in source: raise Fault("unauthorized_scope", "Scheduled proposals publish only within their run", 403)
        current = self.get("interface", proposal["interfaceId"]) if "interfaceId" in proposal else None
        require((current["revision"] if current else 0) == expected, "Interface changed", "revision_conflict", 409)
        provenance = {"dataMode": "synthetic" if source and source["_input"]["grantRefs"] else "nonAccount"}
        if source: provenance["sourceRunId"] = source["id"]
        result = self.write_interface(current, proposal["title"], proposal["spec"], provenance)
        proposal["state"] = "published"; proposal["interfaceId"] = result["id"]; self.put("proposal", pid, proposal)
        if source: self.event(source, "interface.updated", {"interfaceId": result["id"], "revision": result["revision"]})
        return result

    def create_schedule(self, body, source_run=None):
        schedule_input(body); self.workspace(body["workspaceId"]); self.model(body["modelId"])
        target = self.get("interface", body["interfaceId"])
        require(target["revision"] == body["expectedInterfaceRevision"], "Interface changed", "revision_conflict", 409)
        self.check_grants(body["workspaceId"], body["modelId"], body["grantRefs"])
        if source_run:
            self.fence(source_run)
            require("scheduleId" not in source_run and body["workspaceId"] == source_run["workspaceId"]
                    and body["modelId"] == source_run["modelId"] and body["grantRefs"] == source_run["_input"]["grantRefs"],
                    "Schedule proposal outside run scope", "unauthorized_scope", 403)
        schedule = {k: v for k, v in body.items() if k != "expectedInterfaceRevision"}
        schedule.update(id=new_id(), version=1, state="paused", runsStarted=0,
                        interfaceRevisionPolicy="latestAtRunStart", createdAt=stamp(), updatedAt=stamp())
        self.put("schedule", schedule["id"], schedule)
        if source_run: self.event(source_run, "schedule.created", {"scheduleId": schedule["id"], "state": "paused"})
        return schedule

    def pause(self, schedule, reason):
        schedule.update(state="paused", version=schedule["version"] + 1, reason=reason, updatedAt=stamp())
        schedule.pop("nextRunAt", None); self.put("schedule", schedule["id"], schedule)
        self.invalidate(lambda r: r.get("scheduleId") == schedule["id"], reason)

    def invalidate_proposals(self, rid):
        for proposal in self.all("proposal"):
            if proposal.get("sourceRunId") == rid and proposal["state"] == "pending":
                proposal["state"] = "invalidated"; self.put("proposal", proposal["id"], proposal)

    def invalidate(self, predicate, reason):
        for run in self.all("run"):
            if predicate(run):
                self.invalidate_proposals(run["id"])
                if run["state"] not in TERMINAL:
                    self.state(run, "cancelled" if run["state"] == "queued" else "cancelling", reason)

    def tool_response(self, run, action, body):
        obj(body, ("requestId",) if action == "tool-claim" else ("requestId", "claimToken", "outcome"),
            () if action == "tool-claim" else ("data", "error"))
        uid(body["requestId"]); request = self.get("tool", body["requestId"])
        require(request["runId"] == run["id"] and request["deviceId"] == self.device_id, "Tool scope mismatch", "unauthorized_scope", 403)
        self.fence(run)
        require(instant(request["expiresAt"]) > datetime.now(UTC), "Tool request expired", "expired", 410)
        if action == "tool-claim":
            require(request["state"] == "pending" or (request["state"] == "claimed" and instant(request["_claimUntil"]) <= datetime.now(UTC)),
                    "Tool already claimed", "stale_claim", 409)
            token = secrets.token_urlsafe(32)
            until = min(instant(request["expiresAt"]), datetime.now(UTC) + timedelta(seconds=30))
            request.update(state="claimed", _claim=hashlib.sha256(token.encode()).hexdigest(), _claimUntil=stamp(until))
            self.put("tool", request["id"], request)
            return 200, {"request": self.public(request), "claimToken": token, "claimExpiresAt": stamp(until)}
        text(body["claimToken"], 200, 1)
        require(hmac.compare_digest(hashlib.sha256(body["claimToken"].encode()).hexdigest(), request.get("_claim", "")),
                "Claim does not match", "stale_claim", 409)
        digest = hashlib.sha256(canonical(body).encode()).hexdigest()
        if request["state"] == "completed":
            require(request.get("_resultHash") == digest, "Conflicting result", "stale_claim", 409)
            return 200, {"accepted": True, "requestId": request["id"]}
        require(request["state"] == "claimed" and instant(request["_claimUntil"]) > datetime.now(UTC), "Claim expired", "stale_claim", 409)
        outcome = body["outcome"]; require(outcome in ("succeeded", "failed", "denied", "unavailable"), "Invalid outcome")
        if outcome == "succeeded":
            require("data" in body and "error" not in body, "Success needs data only")
            if request["toolName"] in COMPOSIO_TOOLS: composio_result(body["data"], request)
            else: google_result(body["data"], request)
        else:
            require("error" in body and "data" not in body, "Failure needs error only")
            obj(body["error"], ("code", "message")); text(body["error"]["code"], 100, 1); text(body["error"]["message"], 2000)
        request.update(state="completed", _result={k: v for k, v in body.items() if k not in ("claimToken",)}, _resultHash=digest)
        self.put("tool", request["id"], request)
        self.event(run, "tool.result", {"requestId": request["id"], "outcome": outcome})
        self.state(run, "running")
        return 200, {"accepted": True, "requestId": request["id"]}

    def tool(self, rid, name, args):
        """Trusted supervisor RPC only. Identity never comes from model arguments."""
        with self.transaction():
            run = self.fence(self.get("run", rid))
            require(name in TOOLS, "Forbidden tool", "tool_denied", 403)
            run["_toolCalls"] += 1
            require(run["_toolCalls"] <= run["budgets"]["maxToolCalls"], "Tool budget exhausted", "budget_exceeded", 429)
            self.put("run", rid, run)
            if name == "forma_interface_list":
                obj(args)
                # Discovery is metadata only; forma_interface_get carries the
                # individually bounded spec. Keep the existing last-50 contract.
                fields = ("id", "libraryId", "revision", "title", "createdAt", "updatedAt", "provenance")
                result = {"items": [{key: item[key] for key in fields} for item in self.all("interface")[-50:]]}
                require(len(canonical(result).encode()) <= 65536, "Interface metadata budget exceeded", "budget_exceeded", 429)
                return result
            if name == "forma_interface_get":
                obj(args, ("interfaceId",)); uid(args["interfaceId"]); return self.public(self.get("interface", args["interfaceId"]))
            if name == "forma_interface_propose":
                obj(args, ("expectedRevision", "title", "spec"), ("interfaceId",))
                return self.propose(dict(args, workspaceId=run["workspaceId"]), run)
            if name == "forma_schedule_create":
                obj(args, ("interfaceId", "expectedInterfaceRevision", "prompt", "cron", "timezone", "endAt", "maxRuns", "budgets"))
                return self.create_schedule(dict(args, workspaceId=run["workspaceId"], modelId=run["modelId"], grantRefs=run["_input"]["grantRefs"]), run)
            if name in COMPOSIO_TOOLS:
                # Connection-status tools bridge to the native Composio host.
                # They carry no account read authority, so no grant applies;
                # the connection/grant ids are opaque placeholders the host
                # re-validates by run, workspace and device identity.
                if name == COMPOSIO_TOOLS[0]: obj(args)
                else: composio_args(args)
                request = {"id": new_id(), "runId": rid, "workspaceId": run["workspaceId"], "deviceId": self.device_id,
                           "connectionId": new_id(), "grantRef": {"id": new_id(), "generation": 1},
                           "toolName": name, "args": args, "expiresAt": stamp(instant(run["_deadline"])), "state": "pending"}
                self.put("tool", request["id"], request)
                self.event(run, "tool.requested", {"requestId": request["id"], "toolName": name, "deviceId": self.device_id})
                self.state(run, "waiting_for_device", "native_tool_required")
                return {"_waitRequest": request["id"]}
            google_args(args)
            grants = self.check_grants(run["workspaceId"], run["modelId"], run["_input"]["grantRefs"])
            choices = [g for g in grants if name in g["operations"]]
            require(len(choices) == 1, "Exactly one grant for this tool is required", "grant_denied", 403)
            grant = choices[0]
            request = {"id": new_id(), "runId": rid, "workspaceId": run["workspaceId"], "deviceId": grant["deviceId"],
                       "connectionId": grant["connectionId"], "grantRef": {"id": grant["id"], "generation": grant["generation"]},
                       "toolName": name, "args": args, "expiresAt": stamp(min(instant(run["_deadline"]), instant(grant["expiresAt"]))), "state": "pending"}
            self.put("tool", request["id"], request)
            self.event(run, "tool.requested", {"requestId": request["id"], "toolName": name, "deviceId": grant["deviceId"]})
            self.state(run, "waiting_for_device", "native_tool_required")
            return {"_waitRequest": request["id"]}

    def tool_result(self, rid, request_id):
        with self.lock:
            self.fence(self.get("run", rid)); request = self.get("tool", request_id)
            return request.get("_result")

    def start_next(self):
        with self.transaction():
            for run in self.all("run"):
                if run["state"] != "queued": continue
                try: self.fence(run)
                except Fault as exc:
                    self.state(run, "blocked", exc.code); continue
                run["startedAt"] = stamp(); self.state(run, "running"); return run
        return None

    def delta(self, rid, text_delta):
        text(text_delta, 16000)
        with self.transaction():
            run = self.fence(self.get("run", rid))
            require(run["lastSeq"] < 20000, "Event budget exhausted", "budget_exceeded", 429)
            self.event(run, "assistant.delta", {"text": text_delta})

    def finish(self, rid, result=None, failure=None):
        with self.transaction():
            run = self.get("run", rid)
            if run["state"] in TERMINAL: return
            if run["state"] == "cancelling":
                self.state(run, "cancelled", run.get("reason", "cancelled")); self.invalidate_proposals(rid); return
            try:
                self.fence(run)
                if isinstance(failure, Fault): raise failure
                if failure: raise Fault(failure, "Worker did not complete", 409)
                require(isinstance(result, dict) and result.get("completed") is True and not result.get("failed")
                        and not result.get("interrupted") and not result.get("error"), "Hermes did not report success", "hermes_incomplete", 409)
                text(result.get("final_response"), 64000)
                require(len(result["final_response"].encode()) <= 65536, "Final message byte budget exceeded", "budget_exceeded", 429)
                messages = completed_history(result.get("messages"), run)
                require(messages[-1]["content"] == result["final_response"], "Final response differs from committed assistant message")
                pending = [p for p in self.all("proposal") if p.get("sourceRunId") == rid and p["state"] == "pending"]
                if "scheduleId" in run:
                    require(len(pending) == 1, "Scheduled refresh must produce one target revision", "invalid_refresh", 409)
                    self.publish(pending[0]["id"], run["_targetRevision"], scheduled_run=run)
                    # publish emitted an event through a separately loaded run object.
                    run = self.get("run", rid)
                if "scheduleId" not in run:
                    ws = self.get("workspace", run["workspaceId"]); ws["history"] = messages; self.put("workspace", ws["id"], ws)
                run["finalMessage"] = result["final_response"]
                self.event(run, "assistant.message", {"text": result["final_response"]}); self.state(run, "succeeded")
            except Fault as exc:
                self.invalidate_proposals(rid)
                self.state(run, "blocked" if exc.code in ("deadline_exceeded", "grant_denied", "device_unavailable") else "failed", exc.code)
                self.event(run, "error", exc.wire()["error"])
            for request in self.all("tool"):
                if request["runId"] == rid:
                    request.pop("_result", None); request.pop("_claim", None)
                    if request["state"] != "completed": request["state"] = "invalidated"
                    self.put("tool", request["id"], request)

    def fence_model_configuration(self, reason="model_configuration_changed"):
        """Model authority is not transferable across provider/key/catalog changes."""
        with self.transaction():
            self.invalidate(lambda _: True, reason)
            for grant in self.all("grant"):
                if not grant["revoked"]:
                    grant.update(revoked=True, generation=grant["generation"] + 1)
                    self.put("grant", grant["id"], grant)
            for schedule in self.all("schedule"):
                if schedule["state"] == "enabled": self.pause(schedule, reason)
            for proposal in self.all("proposal"):
                if proposal["state"] == "pending":
                    proposal["state"] = "invalidated"; self.put("proposal", proposal["id"], proposal)
            for request in self.all("tool"):
                request.pop("_claim", None); request.pop("_result", None)
                request["state"] = "invalidated"; self.put("tool", request["id"], request)

    def reserve_model_request(self, rid, daily_limit=192):
        """Count every attempt before network egress, including retries and auxiliaries."""
        with self.transaction():
            run = self.fence(self.get("run", rid))
            count = run.get("_modelRequests", 0)
            require(count < run["budgets"]["maxIterations"], "Model request budget exhausted", "budget_exceeded", 429)
            day = datetime.now(UTC).date().isoformat()
            usage = self.get("usage", day, optional=True) or {"day": day, "attempts": 0}
            require(usage["attempts"] < daily_limit, "Daily model request budget exhausted", "daily_budget_exceeded", 429)
            usage["attempts"] += 1; run["_modelRequests"] = count + 1
            self.put("usage", day, usage); self.put("run", rid, run)
            return {"modelId": run["modelId"], "deadline": run["_deadline"], "maxOutputTokens": run["budgets"]["maxOutputTokens"]}

    def recover(self):
        with self.transaction():
            for run in self.all("run"):
                if run["state"] in ("running", "waiting_for_device", "cancelling"):
                    self.state(run, "interrupted", "supervisor_restarted"); self.invalidate_proposals(run["id"])
            for request in self.all("tool"):
                request.pop("_claim", None); request.pop("_result", None)
                if request["state"] in ("pending", "claimed"): request["state"] = "invalidated"
                self.put("tool", request["id"], request)

    def tick(self, now=None):
        now = now or datetime.now(UTC)
        with self.transaction():
            for schedule in self.all("schedule"):
                if schedule["state"] != "enabled": continue
                if instant(schedule["endAt"]) <= now or schedule["runsStarted"] >= schedule["maxRuns"]:
                    # Let an already-admitted final attempt finish within endAt; no new attempt.
                    schedule.update(state="ended", updatedAt=stamp(now)); schedule.pop("nextRunAt", None)
                    self.put("schedule", schedule["id"], schedule); continue
                due_text = schedule.get("nextRunAt")
                if not due_text or instant(due_text) > now: continue
                due = instant(due_text)
                if self.db.execute("SELECT 1 FROM occurrences WHERE schedule_id=? AND slot=?", (schedule["id"], due_text)).fetchone(): continue
                next_due = Cron(schedule["cron"]).next(now, instant(schedule["endAt"]))
                if next_due: schedule["nextRunAt"] = next_due
                else: schedule.pop("nextRunAt", None)
                if now - due >= timedelta(minutes=1):
                    self.db.execute("INSERT INTO occurrences VALUES(?,?,NULL,'missed')", (schedule["id"], due_text))
                else:
                    try:
                        value = {"workspaceId": schedule["workspaceId"], "message": schedule["prompt"], "modelId": schedule["modelId"],
                                 "grantRefs": schedule["grantRefs"], "interfaceIds": [schedule["interfaceId"]], "budgets": schedule["budgets"]}
                        rid = new_id(); self.create_run(value, rid, schedule)
                        schedule["runsStarted"] += 1; schedule["lastRunId"] = rid
                        self.db.execute("INSERT INTO occurrences VALUES(?,?,?,'admitted')", (schedule["id"], due_text, rid))
                    except Fault as exc:
                        self.db.execute("INSERT INTO occurrences VALUES(?,?,NULL,?)", (schedule["id"], due_text, exc.code))
                        schedule["reason"] = exc.code
                schedule["updatedAt"] = stamp(now); self.put("schedule", schedule["id"], schedule)
