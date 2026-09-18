import sys, os; sys.path.insert(0, os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "runtime"))
import tempfile
import unittest
from datetime import datetime, timedelta
from pathlib import Path

from forma_runtime.contracts import UTC, Fault, GOOGLE_TOOLS, instant, new_id, stamp
from forma_runtime.store import Store

LIBRARY = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
DEVICE = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
WORKSPACE = "cccccccc-cccc-4ccc-8ccc-cccccccccccc"
OTHER = "dddddddd-dddd-4ddd-8ddd-dddddddddddd"
CONNECTION = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee"
OTHER_CONNECTION = "f1eeeeee-eeee-4eee-8eee-eeeeeeeeeeee"
BUDGETS = {"maxIterations": 8, "maxToolCalls": 12, "maxOutputTokens": 4096, "maxDurationSeconds": 180}


def spec():
    return {"schemaVersion": 1, "kind": "components", "root": {"id": "root", "type": "stack", "children": [
        {"id": "greeting", "type": "text", "text": "Authority overview"}]}, "datasets": []}


def proposal(wid=WORKSPACE):
    return {"workspaceId": wid, "expectedRevision": 0, "title": "Authority overview", "spec": spec()}


def run_body(wid=WORKSPACE, grants=None):
    return {"workspaceId": wid, "message": "Authority probe", "modelId": "synthetic-model",
            "grantRefs": grants or [], "interfaceIds": [], "budgets": dict(BUDGETS)}


def grant_body(wid=WORKSPACE, connection=CONNECTION, expires_hours=1):
    return {"workspaceId": wid, "connectionId": connection, "deviceId": DEVICE,
            "operations": [GOOGLE_TOOLS[0]], "modelId": "synthetic-model",
            "expiresAt": stamp(datetime.now(UTC) + timedelta(hours=expires_hours)), "dataMode": "synthetic"}


class StoreAuthorityCase(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.path = Path(self.temp.name) / "authority.sqlite3"
        self.store = Store(self.path, LIBRARY, DEVICE, [{"id": "synthetic-model", "available": True}], lambda: True)

    def tearDown(self):
        self.store.close(); self.temp.cleanup()

    def post(self, path, body, key=None):
        return self.store.post(path, key or new_id(), body)[1]

    def admit(self, body=None, rid=None):
        return self.post("/v1/runs", body or run_body(), rid)

    def start(self, body=None):
        run = self.admit(body); self.store.start_next(); return run

    def succeeded_run(self):
        run = self.start()
        self.store.finish(run["id"], result={"completed": True, "messages": [
            {"role": "user", "content": "Authority probe"},
            {"role": "assistant", "content": "Authority answer"}], "final_response": "Authority answer"})
        return run

    def interface(self):
        p = self.post("/v1/interfaces/propose", proposal())
        return self.post("/v1/interfaces/publish", {"proposalId": p["id"], "expectedRevision": 0})

    def schedule(self, interface, end_hours=1, max_runs=3):
        return self.post("/v1/schedules", {"workspaceId": WORKSPACE, "interfaceId": interface["id"],
            "expectedInterfaceRevision": interface["revision"], "prompt": "Authority refresh",
            "modelId": "synthetic-model", "cron": "* * * * *", "timezone": "UTC",
            "endAt": stamp(datetime.now(UTC) + timedelta(hours=end_hours)), "maxRuns": max_runs,
            "budgets": dict(BUDGETS), "grantRefs": []})

    def enable(self, schedule):
        return self.post("/v1/schedules/" + schedule["id"] + "/enable",
                         {"expectedVersion": schedule["version"],
                          "consent": {"scheduleVersion": schedule["version"], "modelId": schedule["modelId"],
                                      "grantRefs": schedule["grantRefs"]}})

    def run_scheduled_refresh(self, enabled, due, interface):
        self.store.tick(due)
        run = self.store.start_next()
        self.assertIsNotNone(run)
        self.assertEqual(run["origin"], "schedule")
        self.store.tool(run["id"], "forma_interface_propose",
                        {"interfaceId": interface["id"], "expectedRevision": interface["revision"],
                         "title": "Authority refreshed", "spec": spec()})
        self.store.finish(run["id"], result={"completed": True, "messages": [
            {"role": "user", "content": "Authority refresh"},
            {"role": "assistant", "content": "Authority refreshed"}], "final_response": "Authority refreshed"})
        self.assertEqual(self.store.get("run", run["id"])["state"], "succeeded")
        return run

    def test_fence_model_configuration_revokes_grants_and_invalidates_pending_runs(self):
        grant_a = self.post("/v1/grants", grant_body(WORKSPACE, CONNECTION))
        grant_b = self.post("/v1/grants", grant_body(OTHER, OTHER_CONNECTION))
        running = self.start(run_body(grants=[{"id": grant_a["id"], "generation": 1}]))
        queued = self.admit(run_body(wid=OTHER, grants=[{"id": grant_b["id"], "generation": 1}]))
        self.store.fence_model_configuration()
        for grant in (grant_a, grant_b):
            record = self.store.get("grant", grant["id"])
            self.assertEqual(record["revoked"], True)
            self.assertEqual(record["generation"], 2)
        self.assertEqual(self.store.get("run", running["id"])["state"], "cancelling")
        cancelled = self.store.get("run", queued["id"])
        self.assertEqual(cancelled["state"], "cancelled")
        self.assertEqual(cancelled["reason"], "model_configuration_changed")

    def test_fence_model_configuration_keeps_terminal_runs_untouched(self):
        done = self.succeeded_run()
        self.store.fence_model_configuration()
        survivor = self.store.get("run", done["id"])
        self.assertEqual(survivor["state"], "succeeded")
        self.assertNotIn("reason", survivor)

    def test_worker_failure_marks_failed_and_terminal_runs_stay_closed(self):
        run = self.start()
        self.store.finish(run["id"], failure="hermes_restart")
        failed = self.store.get("run", run["id"])
        self.assertEqual(failed["state"], "failed")
        self.assertEqual(failed["reason"], "hermes_restart")
        self.store.finish(run["id"], result={"completed": True, "messages": [], "final_response": "late"})
        self.assertEqual(self.store.get("run", run["id"])["state"], "failed")

    def test_enabled_schedule_admits_occurrence_and_last_refresh_ends_schedule(self):
        interface = self.interface()
        schedule = self.schedule(interface, end_hours=1, max_runs=1)
        enabled = self.enable(schedule)
        self.assertEqual(enabled["state"], "enabled")
        self.assertEqual(enabled["version"], schedule["version"] + 1)
        due = instant(enabled["nextRunAt"])
        self.run_scheduled_refresh(enabled, due, interface)
        self.assertEqual(self.store.get("interface", interface["id"])["revision"], 2)
        self.store.tick(due + timedelta(minutes=1))
        self.assertEqual(self.store.get("schedule", schedule["id"])["state"], "ended")
        self.assertEqual(len(self.store.all("run")), 1)

    def test_reenabling_ended_schedule_hits_bounds_without_version_bump(self):
        interface = self.interface()
        schedule = self.schedule(interface, end_hours=1, max_runs=1)
        enabled = self.enable(schedule)
        due = instant(enabled["nextRunAt"])
        self.run_scheduled_refresh(enabled, due, interface)
        self.store.tick(due + timedelta(minutes=1))
        self.assertEqual(self.store.get("schedule", schedule["id"])["state"], "ended")
        with self.assertRaises(Fault) as raised:
            self.enable(enabled)
        self.assertEqual(raised.exception.code, "schedule_ended")
        self.assertEqual(self.store.get("schedule", schedule["id"])["version"], enabled["version"])

    def test_run_admission_is_serialized_per_workspace(self):
        self.start()
        with self.assertRaises(Fault) as raised:
            self.admit()
        self.assertEqual(raised.exception.code, "run_active")

    def test_fenced_enabled_schedule_pauses_once_and_stays_stable(self):
        interface = self.interface()
        schedule = self.enable(self.schedule(interface))
        self.store.fence_model_configuration()
        paused = self.store.get("schedule", schedule["id"])
        self.assertEqual(paused["state"], "paused")
        self.assertEqual(paused["version"], schedule["version"] + 1)
        self.assertEqual(paused["reason"], "model_configuration_changed")
        self.store.fence_model_configuration()
        again = self.store.get("schedule", schedule["id"])
        self.assertEqual(again["state"], "paused")
        self.assertEqual(again["version"], paused["version"])


if __name__ == "__main__":
    unittest.main()
