"""Composio connection-tool contract tests: offline/synthetic, no Composio access.

Run ONLY in the dedicated miniforum target alongside test_runtime.py. Every
tool result here is a synthetic native shape; no key, account, or network use.
"""
import copy
import tempfile
import unittest
from datetime import datetime, timedelta
from pathlib import Path

from forma_runtime.contracts import (COMPOSIO_TOOLS, CONNECTION_STATUS, Fault, GOOGLE_TOOLS, TOOLS,
                                     composio_args, composio_result, google_args, google_result, new_id)
from forma_runtime.server import Application
from forma_runtime.store import Store
from forma_runtime.supervisor import Config
from test_runtime import DEVICE, LIBRARY, WORKSPACE, run_body


def connection_request(slug="github", status="pending_user_action", detail=None):
    data = {"kind": "serviceConnectionRequest", "service": slug, "status": status}
    if detail is not None: data["detail"] = detail
    return data


def services_result(items=None, configured=True):
    return {"kind": "connectedServices", "configured": configured, "items": items or []}


class ComposioContractCase(unittest.TestCase):
    def test_args_shape_is_strict(self):
        self.assertEqual(composio_args({"service": "github"}), {"service": "github"})
        self.assertEqual(composio_args({"service": "google_calendar"}), {"service": "google_calendar"})
        for invalid in ({}, {"service": "GitHub"}, {"service": "-lead"}, {"service": "a" * 81},
                        {"service": "github", "extra": 1}, {"service": "git hub"}):
            with self.assertRaises(Fault): composio_args(invalid)

    def test_result_accepts_honest_statuses_only(self):
        request = {"toolName": COMPOSIO_TOOLS[1], "args": {"service": "github"}}
        for status in CONNECTION_STATUS:
            data = connection_request(status=status)
            self.assertIs(composio_result(data, request), data)
        for invalid in (connection_request(status="connected"),
                        connection_request(status="success"),
                        connection_request(slug="notion"),
                        {"kind": "serviceConnectionRequest", "status": "pending_user_action"},
                        {"kind": "connectedServices", "status": "pending_user_action", "service": "github"}):
            with self.assertRaises(Fault): composio_result(invalid, request)
        detail = connection_request(status="failed", detail="Not configured on this device yet.")
        self.assertIs(composio_result(detail, request), detail)
        with self.assertRaises(Fault): composio_result(connection_request(status="failed", detail="x" * 201), request)
        with self.assertRaises(Fault): composio_result(connection_request(status="failed", detail=7), request)

    def test_list_result_shape_and_limits(self):
        request = {"toolName": COMPOSIO_TOOLS[0], "args": {}}
        base = services_result()
        self.assertIs(composio_result(base, request), base)
        items = [{"slug": f"service_{index}", "status": "connected"} for index in range(50)]
        self.assertEqual(len(composio_result(services_result(items), request)["items"]), 50)
        with self.assertRaises(Fault): composio_result(services_result(items * 2), request)
        with self.assertRaises(Fault): composio_result(services_result([{"slug": "github", "status": "live"}]), request)
        with self.assertRaises(Fault): composio_result(services_result([{"slug": "GitHub", "status": "connected"}]), request)
        with self.assertRaises(Fault): composio_result(services_result([{"slug": "github"}]), request)
        with self.assertRaises(Fault): composio_result({"kind": "connectedServices", "configured": "yes", "items": []}, request)
        with self.assertRaises(Fault): composio_result({"kind": "serviceConnectionRequest", "service": "github",
                                                        "status": "pending_user_action"}, request)
        # The two tool families never accept each other's payloads.
        google_request = {"toolName": GOOGLE_TOOLS[0], "args": {"startAt": "2026-09-10T00:00:00Z",
                                                                "endAt": "2026-09-11T00:00:00Z", "maxItems": 1}}
        google_args(google_request["args"])
        with self.assertRaises(Fault): composio_result(base, google_request)

    def test_allowlist_and_google_tools_unchanged(self):
        self.assertIn(COMPOSIO_TOOLS[0], TOOLS)
        self.assertIn(COMPOSIO_TOOLS[1], TOOLS)
        self.assertEqual(TOOLS.count(COMPOSIO_TOOLS[0]), 1)
        self.assertEqual(len(TOOLS), 8)
        # A google tool result must still be validated by google_result.
        request = {"toolName": GOOGLE_TOOLS[1], "args": {"startAt": "2026-09-10T00:00:00Z",
                                                         "endAt": "2026-09-11T00:00:00Z", "maxItems": 1}}
        google_args(request["args"])
        with self.assertRaises(Fault): google_result(services_result(), request)


class ComposioToolCase(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.path = Path(self.temp.name) / "test.sqlite3"
        self.store = Store(self.path, LIBRARY, DEVICE, [{"id": "synthetic-model", "available": True}], lambda: True)

    def tearDown(self):
        self.store.close(); self.temp.cleanup()

    def post(self, path, body, key=None):
        return self.store.post(path, key or new_id(), body)[1]

    def start(self):
        run = self.post("/v1/runs", run_body())
        self.store.start_next()
        return run

    def test_connection_tool_waits_for_device_without_grant(self):
        run = self.start()
        wait = self.store.tool(run["id"], COMPOSIO_TOOLS[1], {"service": "github"})
        self.assertIn("_waitRequest", wait)
        record = self.store.get("tool", wait["_waitRequest"])
        self.assertEqual(record["toolName"], COMPOSIO_TOOLS[1])
        self.assertEqual(record["workspaceId"], WORKSPACE)
        self.assertEqual(record["deviceId"], DEVICE)
        self.assertEqual(record["state"], "pending")
        self.assertEqual(record["args"], {"service": "github"})
        self.assertEqual(self.store.get("run", run["id"])["state"], "waiting_for_device")
        # No grant was needed and none exists.
        self.assertEqual(run_body()["grantRefs"], [])

    def test_connection_tool_args_are_validated_before_any_request(self):
        run = self.start()
        for invalid in ({}, {"service": "GitHub"}, {"service": "github", "extra": 1}):
            with self.assertRaises(Fault): self.store.tool(run["id"], COMPOSIO_TOOLS[1], invalid)
        with self.assertRaises(Fault): self.store.tool(run["id"], COMPOSIO_TOOLS[0], {"service": "github"})
        self.assertEqual(self.store.get("run", run["id"])["state"], "running")

    def test_native_claim_and_result_round_trip(self):
        for tool, args, data in (
            (COMPOSIO_TOOLS[0], {}, services_result([{"slug": "github", "status": "connected"}])),
            (COMPOSIO_TOOLS[1], {"service": "linear"}, connection_request(slug="linear", status="pending_user_action")),
        ):
            with self.subTest(tool=tool):
                run = self.start()
                wait = self.store.tool(run["id"], tool, args)
                request_id = wait["_waitRequest"]
                claim = self.post(f'/v1/runs/{run["id"]}/tool-claim', {"requestId": request_id})
                result = {"requestId": request_id, "claimToken": claim["claimToken"],
                          "outcome": "succeeded", "data": copy.deepcopy(data)}
                malformed = copy.deepcopy(result)
                malformed["data"]["kind"] = "wrong"
                with self.assertRaises(Fault): self.post(f'/v1/runs/{run["id"]}/tool-result', malformed)
                self.post(f'/v1/runs/{run["id"]}/tool-result', result)
                self.assertEqual(self.store.tool_result(run["id"], request_id)["data"], data)
                self.assertEqual(self.store.get("run", run["id"])["state"], "running")
                self.store.finish(run["id"], result={"completed": True, "messages": [], "final_response": "done"})

    def test_unavailable_outcome_reports_denial_not_success(self):
        run = self.start()
        wait = self.store.tool(run["id"], COMPOSIO_TOOLS[1], {"service": "github"})
        claim = self.post(f'/v1/runs/{run["id"]}/tool-claim', {"requestId": wait["_waitRequest"]})
        result = {"requestId": wait["_waitRequest"], "claimToken": claim["claimToken"], "outcome": "unavailable",
                  "error": {"code": "native_tool_unavailable", "message": "The connector host is not configured."}}
        self.post(f'/v1/runs/{run["id"]}/tool-result', result)
        delivered = self.store.tool_result(run["id"], wait["_waitRequest"])
        self.assertEqual(delivered["outcome"], "unavailable")
        self.assertNotIn("data", delivered)


class ComposioCapabilityCase(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        config = Config(database=Path(self.temp.name) / "runtime.sqlite3", library_id=LIBRARY, device_id=DEVICE,
                        token_sha256="", models=[{"id": "synthetic-model", "name": "Synthetic", "available": True}])
        self.app = Application(config)

    def tearDown(self):
        self.app.close(); self.temp.cleanup()

    def test_composio_tools_are_native_device_and_need_no_grant(self):
        tools = {tool["name"]: tool for tool in self.app.capabilities()["tools"]}
        for name in COMPOSIO_TOOLS:
            self.assertIn(name, tools)
            self.assertEqual(tools[name]["execution"], "nativeDevice")
            self.assertNotIn("reason", tools[name])
        self.assertEqual(tools[GOOGLE_TOOLS[0]]["execution"], "nativeDevice")
        self.assertIn("reason", tools[GOOGLE_TOOLS[0]])
        self.assertEqual(tools["forma_interface_list"]["execution"], "runtime")


class WorkerSchemaCase(unittest.TestCase):
    def test_worker_registers_direct_composio_schemas(self):
        # worker.py redirects stdout at import time; inspect the pinned source
        # instead of executing it. The definitions must cover the allowlist.
        source = (Path(__file__).resolve().parent.parent / "forma_runtime" / "worker.py").read_text()
        self.assertIn("COMPOSIO_TOOLS", source)
        self.assertIn('schema(COMPOSIO_TOOLS[0], {})', source)
        self.assertIn('schema(COMPOSIO_TOOLS[1], {"service": string}, ("service",))', source)
        self.assertIn('"forma_request_service_connection"', source)
        self.assertIn('"forma_list_connected_services"', source)


if __name__ == "__main__":
    unittest.main()
