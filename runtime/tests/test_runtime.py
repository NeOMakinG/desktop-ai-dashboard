"""Offline/synthetic tests. Run ONLY in the dedicated miniforum target.

No Hermes imports, model calls, account access, active service or live schedules.
"""
import ast
import copy
import hashlib
import http.client
import json
import tempfile
import threading
import unittest
from datetime import datetime, timedelta
from pathlib import Path

from forma_runtime.contracts import *
from forma_runtime.server import Application, Server
from forma_runtime.store import Store
from forma_runtime.supervisor import Config, sandbox_command

LIBRARY = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
DEVICE = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
WORKSPACE = "cccccccc-cccc-4ccc-8ccc-cccccccccccc"
OTHER = "dddddddd-dddd-4ddd-8ddd-dddddddddddd"
CONNECTION = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee"
BUDGETS = {"maxIterations": 8, "maxToolCalls": 12, "maxOutputTokens": 4096, "maxDurationSeconds": 180}
TOKEN = "synthetic-test-credential-not-an-account-key"


def spec():
    return {"schemaVersion": 1, "kind": "components", "root": {"id": "root", "type": "stack", "children": [
        {"id": "greeting", "type": "text", "text": "Synthetic overview"}]}, "datasets": []}


def run_body(wid=WORKSPACE, grants=None):
    return {"workspaceId": wid, "message": "Synthetic request", "modelId": "synthetic-model",
            "grantRefs": grants or [], "interfaceIds": [], "budgets": dict(BUDGETS)}


def proposal(wid=WORKSPACE):
    return {"workspaceId": wid, "expectedRevision": 0, "title": "Synthetic overview", "spec": spec()}


class WorkerTransportCase(unittest.TestCase):
    def test_client_and_transport_ignore_ambient_certificate_configuration(self):
        source = Path(__file__).resolve().parents[1] / "forma_runtime" / "worker.py"
        tree = ast.parse(source.read_text())
        checked = set()
        for node in ast.walk(tree):
            if not (isinstance(node, ast.Call) and isinstance(node.func, ast.Attribute)
                    and isinstance(node.func.value, ast.Name) and node.func.value.id == "httpx"
                    and node.func.attr in {"Client", "HTTPTransport"}):
                continue
            options = {item.arg: item.value for item in node.keywords}
            self.assertIn("trust_env", options, node.func.attr)
            self.assertIsInstance(options["trust_env"], ast.Constant)
            self.assertIs(options["trust_env"].value, False)
            if "verify" in options:
                self.assertFalse(isinstance(options["verify"], ast.Constant)
                                 and options["verify"].value is False)
            checked.add(node.func.attr)
        self.assertEqual(checked, {"Client", "HTTPTransport"})


class StoreCase(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.path = Path(self.temp.name) / "test.sqlite3"
        self.store = Store(self.path, LIBRARY, DEVICE, [{"id": "synthetic-model", "available": True}], lambda: True)

    def tearDown(self):
        self.store.close(); self.temp.cleanup()

    def post(self, path, body, key=None):
        return self.store.post(path, key or new_id(), body)[1]

    def admit(self, body=None): return self.post("/v1/runs", body or run_body())

    def start(self, body=None):
        run = self.admit(body); self.store.start_next(); return run

    def interface(self):
        p = self.post("/v1/interfaces/propose", proposal())
        return self.post("/v1/interfaces/publish", {"proposalId": p["id"], "expectedRevision": 0})

    def schedule(self, interface):
        return self.post("/v1/schedules", {"workspaceId": WORKSPACE, "interfaceId": interface["id"],
            "expectedInterfaceRevision": interface["revision"], "prompt": "Synthetic refresh", "modelId": "synthetic-model",
            "cron": "* * * * *", "timezone": "UTC", "endAt": stamp(datetime.now(UTC) + timedelta(hours=1)),
            "maxRuns": 2, "budgets": dict(BUDGETS), "grantRefs": []})

    def enable(self, schedule):
        return self.post(f'/v1/schedules/{schedule["id"]}/enable', {"expectedVersion": schedule["version"],
            "consent": {"scheduleVersion": schedule["version"], "modelId": schedule["modelId"], "grantRefs": schedule["grantRefs"]}})

    def test_idempotent_run_identity_history_and_conflicting_body(self):
        key = new_id(); body = run_body(); body["importHistory"] = [{"role": "user", "content": "Old"}]
        first = self.post("/v1/runs", body, key)
        self.assertEqual(first, self.post("/v1/runs", body, key)); self.assertEqual(first["id"], key)
        with self.assertRaises(Fault): self.post("/v1/runs", dict(body, message="different"), key)
        self.assertEqual(len(self.store.all("run")), 1)
        self.post(f"/v1/runs/{key}/cancel", {})
        with self.assertRaises(Fault) as raised: self.admit(body)
        self.assertEqual(raised.exception.code, "history_already_initialized")
        self.admit(run_body())
        self.assertEqual(self.store.get("workspace", WORKSPACE)["history"], body["importHistory"])

    def test_workspace_precreation_does_not_block_first_history_import(self):
        self.interface()
        body = run_body(); body["importHistory"] = [{"role": "assistant", "content": "Existing local reply"}]
        self.admit(body)
        self.assertEqual(self.store.get("workspace", WORKSPACE)["history"], body["importHistory"])

    def test_single_database_owner_and_safe_journal(self):
        with self.assertRaises(Fault) as raised:
            Store(self.path, LIBRARY, DEVICE, [], lambda: False)
        self.assertEqual(raised.exception.code, "database_in_use")
        self.assertEqual(self.store.db.execute("PRAGMA journal_mode").fetchone()[0], "delete")
        self.assertEqual(self.store.db.execute("PRAGMA synchronous").fetchone()[0], 2)

    def test_delete_before_workspace_admission_fences_late_run(self):
        receipt = self.post(f"/v1/workspaces/{WORKSPACE}/delete", {})
        self.assertEqual(receipt["id"], WORKSPACE)
        with self.assertRaises(Fault) as raised: self.admit()
        self.assertEqual(raised.exception.status, 410)

    def test_cancel_before_admission_is_durable_fence(self):
        rid = new_id()
        receipt = self.post(f"/v1/runs/{rid}/cancel", {})
        self.assertEqual(receipt, {"id": rid, "state": "cancelled", "admitted": False})
        with self.assertRaises(Fault) as raised: self.post("/v1/runs", run_body(), rid)
        self.assertEqual(raised.exception.code, "run_cancelled_before_admission")
        self.store.close(); self.store = Store(self.path, LIBRARY, DEVICE, [{"id": "synthetic-model"}], lambda: True)
        with self.assertRaises(Fault): self.post("/v1/runs", run_body(), rid)
        self.assertEqual(self.store.all("run"), [])

    def test_last_schedule_attempt_publishes_and_does_not_grow_human_history(self):
        interface = self.interface(); schedule = self.schedule(interface)
        with self.store.transaction():
            schedule["maxRuns"] = 1; self.store.put("schedule", schedule["id"], schedule)
            ws = self.store.get("workspace", WORKSPACE); ws["history"] = [{"role": "user", "content": "Human conversation"}]
            self.store.put("workspace", WORKSPACE, ws)
        schedule = self.enable(schedule); due = instant(schedule["nextRunAt"])
        self.store.tick(due); self.store.tick(due)
        run = self.store.start_next()
        self.store.tool(run["id"], "forma_interface_propose", {"interfaceId": interface["id"], "expectedRevision": 1,
            "title": "Synthetic scheduled update", "spec": spec()})
        self.store.finish(run["id"], result={"completed": True, "messages": [
            {"role": "user", "content": "Synthetic refresh"}, {"role": "assistant", "content": "Synthetic refreshed"}],
            "final_response": "Synthetic refreshed"})
        self.assertEqual(self.store.get("run", run["id"])["state"], "succeeded")
        self.assertEqual(self.store.get("interface", interface["id"])["revision"], 2)
        self.assertEqual(self.store.get("workspace", WORKSPACE)["history"], [{"role": "user", "content": "Human conversation"}])
        self.assertEqual(self.store.get("schedule", schedule["id"])["state"], "ended")
        self.store.tick(due + timedelta(minutes=1)); self.assertEqual(len(self.store.all("run")), 1)

    def test_event_replay_two_observers_and_paged_cursor(self):
        run = self.start()
        self.store.delta(run["id"], "one"); self.store.delta(run["id"], "two")
        path = f'/v1/runs/{run["id"]}'
        a = self.store.get_route(path, {"after": "0", "limit": "2"})
        b = self.store.get_route(path, {"after": "0", "limit": "2"})
        self.assertEqual(a, b); self.assertTrue(a["hasMore"]); self.assertEqual(a["nextAfter"], 2)
        c = self.store.get_route(path, {"after": str(a["nextAfter"]), "limit": "2"})
        self.assertEqual([x["seq"] for x in c["events"]], [3, 4])

    def test_cancel_waits_for_reap_and_fences_proposals(self):
        run = self.start()
        pending = self.store.tool(run["id"], "forma_interface_propose", {k: v for k, v in proposal().items() if k != "workspaceId"})
        result = self.post(f'/v1/runs/{run["id"]}/cancel', {})
        self.assertEqual(result["state"], "cancelling")
        self.store.finish(run["id"], result={"completed": True, "messages": [], "final_response": "late"})
        self.assertEqual(self.store.get("run", run["id"])["state"], "cancelled")
        with self.assertRaises(Fault): self.post("/v1/interfaces/publish", {"proposalId": pending["id"], "expectedRevision": 0})
        self.assertEqual(self.store.all("interface"), [])

    def test_tombstone_shared_interface_and_revision_cas(self):
        interface = self.interface()
        update = proposal(OTHER); update.update(interfaceId=interface["id"], expectedRevision=1)
        p = self.post("/v1/interfaces/propose", update)
        renamed = self.post(f'/v1/interfaces/{interface["id"]}/rename', {"expectedRevision": 1, "title": "New title"})
        self.assertEqual(renamed["revision"], 2)
        with self.assertRaises(Fault): self.post("/v1/interfaces/publish", {"proposalId": p["id"], "expectedRevision": 1})
        self.post(f'/v1/workspaces/{WORKSPACE}/delete', {})
        self.assertEqual(self.store.get("interface", interface["id"])["title"], "New title")
        with self.assertRaises(Fault): self.admit()
        self.post(f'/v1/interfaces/{interface["id"]}/delete', {"expectedRevision": 2})
        with self.assertRaises(Fault): self.store.get("interface", interface["id"])

    def test_list_summaries_and_detail(self):
        for _ in range(3): self.interface()
        page = self.store.get_route("/v1/interfaces", {"workspaceId": WORKSPACE, "limit": "2"})
        self.assertEqual(len(page["items"]), 2); self.assertTrue(page["hasMore"])
        self.assertNotIn("spec", page["items"][0])
        detail = self.store.get_route("/v1/interfaces/" + page["items"][0]["id"], {})
        self.assertIn("spec", detail)
        second = self.store.get_route("/v1/interfaces", {"workspaceId": WORKSPACE, "limit": "2", "cursor": page["nextCursor"]})
        self.assertEqual(len(second["items"]), 1); self.assertFalse(second["hasMore"])

    def test_schedule_paused_recovery_unique_occurrence_and_pause_fence(self):
        schedule = self.schedule(self.interface()); self.assertEqual(schedule["state"], "paused")
        self.store.tick(); self.assertEqual(self.store.all("run"), [])
        schedule = self.enable(schedule); due = instant(schedule["nextRunAt"])
        self.store.tick(due); self.store.tick(due)
        runs = self.store.all("run"); self.assertEqual(len(runs), 1)
        self.assertEqual(runs[0]["origin"], "schedule"); self.assertEqual(runs[0]["_history"], [])
        self.store.start_next()
        self.post(f'/v1/schedules/{schedule["id"]}/pause', {"expectedVersion": schedule["version"]})
        self.store.finish(runs[0]["id"], result={"completed": True, "messages": [], "final_response": "late"})
        self.assertEqual(self.store.get("run", runs[0]["id"])["state"], "cancelled")
        self.store.close(); self.store = Store(self.path, LIBRARY, DEVICE, [{"id": "synthetic-model"}], lambda: True)
        self.store.recover(); self.store.tick(due + timedelta(minutes=3))
        self.assertEqual(self.store.get("schedule", schedule["id"])["state"], "paused")
        self.assertEqual(len(self.store.all("run")), 1)

    def test_restart_interrupts_and_does_not_catch_up(self):
        schedule = self.enable(self.schedule(self.interface()))
        self.store.tick(instant(schedule["nextRunAt"]) + timedelta(minutes=5))
        self.assertFalse(self.store.all("run"))
        run = self.start(); self.store.recover()
        self.assertEqual(self.store.get("run", run["id"])["state"], "interrupted")

    def test_model_request_budgets_count_attempts(self):
        run = self.start()
        self.store.reserve_model_request(run["id"], daily_limit=2)
        self.store.reserve_model_request(run["id"], daily_limit=2)
        with self.assertRaises(Fault) as raised: self.store.reserve_model_request(run["id"], daily_limit=2)
        self.assertEqual(raised.exception.code, "daily_budget_exceeded")
        self.assertEqual(self.store.get("run", run["id"])["_modelRequests"], 2)

    def test_bridge_claim_duplicate_revoke_no_live_grant(self):
        body = {"workspaceId": WORKSPACE, "connectionId": CONNECTION, "deviceId": DEVICE,
                "operations": [GOOGLE_TOOLS[0]], "modelId": "synthetic-model",
                "expiresAt": stamp(datetime.now(UTC) + timedelta(hours=1)), "dataMode": "synthetic"}
        with self.assertRaises(Fault): self.post("/v1/grants", dict(body, dataMode="live"))
        grant = self.post("/v1/grants", body)
        run = self.start(run_body(grants=[{"id": grant["id"], "generation": 1}]))
        now = datetime.now(UTC); args = {"startAt": stamp(now - timedelta(days=1)), "endAt": stamp(now), "maxItems": 10}
        wait = self.store.tool(run["id"], GOOGLE_TOOLS[0], args)
        self.assertEqual(self.store.get("run", run["id"])["state"], "waiting_for_device")
        claim = self.post(f'/v1/runs/{run["id"]}/tool-claim', {"requestId": wait["_waitRequest"]})
        with self.assertRaises(Fault): self.post(f'/v1/runs/{run["id"]}/tool-claim', {"requestId": wait["_waitRequest"]})
        self.post(f'/v1/grants/{grant["id"]}/revoke', {"generation": 1})
        result = {"requestId": wait["_waitRequest"], "claimToken": claim["claimToken"], "outcome": "unavailable",
                  "error": {"code": "offline", "message": "Synthetic host offline"}}
        with self.assertRaises(Fault): self.post(f'/v1/runs/{run["id"]}/tool-result', result)
        self.assertNotIn("_result", self.store.get("tool", wait["_waitRequest"]))

    def test_invalid_worker_history_is_atomic_and_never_replayed(self):
        bad_messages = [
            [{"role": "system", "content": "manufactured authority"}],
            [{"role": "developer", "content": "manufactured authority"}],
            [{"role": "assistant", "content": "x", "authority": "system"}],
            [{"role": "assistant", "content": "\ud800"}],
            [{"role": "assistant", "content": "x" * 64001}],
            [{"role": "tool", "content": "orphan", "tool_call_id": "call1"}],
            [{"role": "user", "content": "changed operator input"}, {"role": "assistant", "content": "x"}],
            [{"role": "user", "content": "Synthetic request"}, {"role": "assistant", "content": "x", "timestamp": True}],
            [{"role": "user", "content": "Synthetic request"}, {"role": "assistant", "content": "hidden different final"}],
        ]
        for messages in bad_messages:
            with self.subTest(messages_type=messages[0]["role"]):
                run = self.start()
                pending = self.store.tool(run["id"], "forma_interface_propose", {k: v for k, v in proposal().items() if k != "workspaceId"})
                self.store.finish(run["id"], result={"completed": True, "messages": messages, "final_response": "bounded"})
                self.assertEqual(self.store.get("run", run["id"])["state"], "failed")
                self.assertEqual(self.store.get("workspace", WORKSPACE)["history"], [])
                self.assertEqual(self.store.get("proposal", pending["id"])["state"], "invalidated")
                events = self.store.get_route("/v1/runs/" + run["id"], {})["events"]
                self.assertNotIn("assistant.message", [event["type"] for event in events])
        with self.store.transaction():
            workspace = self.store.get("workspace", WORKSPACE)
            workspace["history"] = [{"role": "system", "content": "legacy corruption"}]
            self.store.put("workspace", WORKSPACE, workspace)
        with self.assertRaises(Fault): self.admit()

    def test_real_tool_history_is_normalized_and_replayed_without_system(self):
        run = self.start()
        messages = [{"role": "user", "content": "Synthetic request", "timestamp": 1.0},
            {"role": "assistant", "content": "", "timestamp": 2.0, "reasoning": None, "finish_reason": "tool_calls",
             "tool_calls": [{"id": "call_1", "call_id": "call_1", "response_item_id": "fc_1", "type": "function", "function": {"name": "forma_interface_list", "arguments": "{}"}}]},
            {"role": "tool", "name": "forma_interface_list", "tool_name": "forma_interface_list", "timestamp": 3.0,
             "tool_call_id": "call_1", "content": '{"items":[]}'},
            {"role": "assistant", "content": "Done", "reasoning": None, "finish_reason": "stop", "timestamp": 4.0}]
        self.store.finish(run["id"], result={"completed": True, "messages": messages, "final_response": "Done"})
        self.assertEqual(self.store.get("run", run["id"])["state"], "succeeded")
        normalized = self.store.get("workspace", WORKSPACE)["history"]
        self.assertEqual(normalized, runtime_history(messages))
        self.assertNotIn("timestamp", canonical(normalized))
        next_run = self.start()
        self.assertEqual(self.store.get("run", next_run["id"])["_history"], normalized)
        changed = copy.deepcopy(normalized); changed[0]["content"] = "Changed old operator message"
        changed.extend([{"role": "user", "content": "Synthetic request"}, {"role": "assistant", "content": "Done"}])
        self.store.finish(next_run["id"], result={"completed": True, "messages": changed, "final_response": "Done"})
        self.assertEqual(self.store.get("run", next_run["id"])["state"], "failed")
        self.assertEqual(self.store.get("workspace", WORKSPACE)["history"], normalized)

    def test_library_operator_admin_is_not_a_workspace_tenant_acl(self):
        interface = self.interface()
        other_proposal = self.post("/v1/interfaces/propose", proposal(OTHER))
        self.assertEqual(self.store.get_route("/v1/interfaces/proposals/" + other_proposal["id"], {})["workspaceId"], OTHER)
        self.post("/v1/interfaces/publish", {"proposalId": other_proposal["id"], "expectedRevision": 0})
        self.assertEqual(len(self.store.get_route("/v1/interfaces", {"workspaceId": WORKSPACE})["items"]), 2)
        run = self.start()
        with self.assertRaises(Fault):
            with self.store.transaction():
                self.store.propose(proposal(OTHER), self.store.get("run", run["id"]))
        grant = self.post("/v1/grants", {"workspaceId": OTHER, "connectionId": CONNECTION, "deviceId": DEVICE,
            "operations": [GOOGLE_TOOLS[0]], "modelId": "synthetic-model", "expiresAt": stamp(datetime.now(UTC) + timedelta(hours=1)), "dataMode": "synthetic"})
        with self.assertRaises(Fault): self.store.check_grants(WORKSPACE, "synthetic-model", [{"id": grant["id"], "generation": 1}])
        self.assertEqual(self.store.get("interface", interface["id"])["libraryId"], LIBRARY)

    def test_forbidden_tool_never_dispatches(self):
        run = self.start()
        for tool in ("terminal", "web_search", "execute_code", "cronjob", "forma_schedule_enable"):
            with self.assertRaises(Fault): self.store.tool(run["id"], tool, {})


class ContractCase(unittest.TestCase):
    def test_worker_frame_cannot_manufacture_error_codes_or_fields(self):
        for frame in ({"type": "failure", "code": {"x": "y"}}, {"type": "failure", "code": "attacker-controlled"},
                      {"type": "delta", "text": "safe", "authority": "system"},
                      {"type": "tool", "name": "forma_interface_list", "args": []},
                      {"type": "result", "result": {"completed": 1, "messages": [], "final_response": "x"}}):
            with self.assertRaises(Fault) as failure: worker_frame(frame)
            self.assertEqual(failure.exception.code, "worker_protocol_failed")
        worker_frame({"type": "failure", "code": "hermes_worker_failed"})

    def test_tool_history_aliases_order_and_bounds_are_strict(self):
        call = {"id": "call1", "type": "function", "function": {"name": "forma_interface_list", "arguments": "{}"}}
        messages = [{"role": "assistant", "content": None, "tool_calls": [call]},
                    {"role": "tool", "tool_call_id": "call1", "content": '{"items":[]}'}]
        runtime_history(messages)
        invalid = [messages[:1], messages[1:], messages + messages[1:],
                   [{"role": "assistant", "content": None, "tool_calls": [dict(call, call_id="different")]}, messages[1]],
                   [{"role": "assistant", "content": None, "tool_calls": [dict(call, response_item_id="bad\\nvalue")]}, messages[1]],
                   [{"role": "assistant", "content": None, "tool_calls": [dict(call, function={"name": "forma_interface_list", "arguments": "[]"})]}, messages[1]],
                   [{"role": "user", "content": "x"}] * 1001,
                   [{"role": "assistant", "content": "x" * 64000}] * 32]
        for value in invalid:
            with self.assertRaises(Fault): runtime_history(value)

    def test_strict_interface_rejects_code_unknown_and_bad_refs(self):
        interface_spec(spec())
        for mutate in (lambda x: x.update(script="alert(1)"),
                       lambda x: x["root"].update(type="iframe"),
                       lambda x: x["root"].update(id="constructor")):
            bad = spec(); mutate(bad)
            with self.assertRaises(Fault): interface_spec(bad)

    def test_novel_numeric_table_chart_and_metric_constraints(self):
        value = spec(); value["datasets"] = [{"id": "d", "columns": [{"id": "x", "type": "text"}, {"id": "y", "type": "number"}],
                                              "rows": [{"x": "A", "y": 2}, {"x": "B", "y": None}]}]
        value["root"]["children"] = [{"id": "chart", "type": "chart", "kind": "bar", "datasetId": "d", "xField": "x", "yField": "y", "label": "Synthetic", "units": "count"}]
        interface_spec(value)
        bad = copy.deepcopy(value); bad["datasets"][0]["rows"][1]["x"] = "A"
        with self.assertRaises(Fault): interface_spec(bad)
        bad = copy.deepcopy(value); bad["root"]["children"] = [{"id": "m", "type": "metric", "label": "Count", "datasetId": "d", "field": "y", "format": "number"}]
        with self.assertRaises(Fault): interface_spec(bad)

    def test_cron_utc_semantics(self):
        at = instant("2026-09-10T12:00:00Z")
        self.assertEqual(Cron("*/15 * * * *").next(at, at + timedelta(hours=1)), "2026-09-10T12:15:00.000Z")
        self.assertTrue(Cron("0 12 1 * 4").matches(at))  # Thursday OR first day.
        for bad in ("* * * * * *", "@hourly", "60 * * * *", "*/0 * * * *", "0 0 * JAN *"):
            with self.assertRaises(Fault): Cron(bad)

    def test_duplicate_json_fields_and_nonfinite_rejected(self):
        for raw in ('{"a":1,"a":2}', '{"x":NaN}'):
            with self.assertRaises((Fault, ValueError)): decode(raw)


class ReflectionCase(unittest.TestCase):
    def test_escaped_json_and_split_sse_secret_are_rejected(self):
        from forma_runtime.supervisor import guard_model_response
        secret = "a" * 32
        escaped = "".join("\\u%04x" % ord(ch) for ch in secret)
        raw = ('{"content":"' + escaped + '"}').encode()
        with self.assertRaises(Fault): guard_model_response(raw, "application/json", secret)
        for field in ("content", "arguments"):
            fragments = [secret[:8], secret[8:]] if field == "content" else [escaped[:7], escaped[7:]]
            events = []
            for fragment in fragments:
                delta = {"content": fragment} if field == "content" else {"tool_calls": [{"index": 0, "function": {"arguments": fragment}}]}
                events.append("data: " + json.dumps({"choices": [{"index": 0, "delta": delta}]}) + "\n\n")
            for separator in ("\n", "\r\n", "\r"):
                framed = (": keepalive\n\n" + "".join(events)).replace("\n", separator)
                with self.subTest(field=field, separator=repr(separator)):
                    with self.assertRaises(Fault): guard_model_response(framed.encode(), "text/event-stream", secret)
        guard_model_response(b'{"content":"Harmless synthetic response"}', "application/json", secret)

    def test_sse_reconstruction_indices_are_strict_bounded_integers(self):
        from forma_runtime.supervisor import guard_model_response
        for invalid in ("0", 0.0, True, None, -1, 16):
            for field in ("choice", "tool"):
                choice = {"index": invalid if field == "choice" else 0,
                          "delta": {"tool_calls": [{"index": invalid if field == "tool" else 0,
                                                   "function": {"arguments": "harmless"}}]}}
                raw = ("data: " + json.dumps({"choices": [choice]}) + "\n\n").encode()
                with self.subTest(field=field, invalid=invalid):
                    with self.assertRaises(Fault): guard_model_response(raw, "text/event-stream", "synthetic-secret")
        for separator in ("\n", "\r\n", "\r"):
            raw = (": keepalive" + separator * 2 + 'data: {"choices":[{"index":0,"delta":{"content":"safe"}}]}' + separator * 2).encode()
            guard_model_response(raw, "text/event-stream", "synthetic-secret")


class HttpCase(unittest.TestCase):
    def test_synthetic_http_auth_and_protocol_receipt(self):
        with tempfile.TemporaryDirectory() as directory:
            config = Config(Path(directory) / "test.db", LIBRARY, DEVICE, hashlib.sha256(TOKEN.encode()).hexdigest(),
                            [{"id": "synthetic-model", "name": "Synthetic offline model", "available": True}])
            app = Application(config); server = Server(("127.0.0.1", 0), app)
            thread = threading.Thread(target=server.serve_forever, daemon=True); thread.start()
            try:
                client = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=3)
                client.request("GET", "/v1/capabilities"); response = client.getresponse(); response.read()
                self.assertEqual(response.status, 401)
                client = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=3)
                client.request("GET", "/v1/capabilities", headers={"Authorization": "Bearer " + TOKEN})
                response = client.getresponse(); payload = json.loads(response.read())
                self.assertEqual(response.status, 200); self.assertFalse(payload["runtime"]["ready"])
                self.assertFalse(payload["features"]["liveGoogle"])
                self.assertEqual(payload["runtime"]["revision"], REVISION)
            finally:
                server.shutdown(); server.server_close(); app.close(); thread.join(timeout=2)


if __name__ == "__main__": unittest.main()
