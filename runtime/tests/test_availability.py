"""Deterministic availability regressions; execute only on dedicated miniforum.

All sockets are synthetic loopback/UDS fixtures. Worker fixtures use stdlib only;
no model, Hermes, account, active QA database, or live schedule is accessed.
"""
import hashlib
import http.client
import http.server
import json
import socket
import select
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from datetime import datetime, timedelta
from pathlib import Path
from unittest.mock import patch

from forma_runtime.contracts import *
from forma_runtime.server import Application, Server
from forma_runtime.store import Store
from forma_runtime.supervisor import Config, RelayHandler, RelayServer, Supervisor, MAX_FRAME

LIBRARY = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
DEVICE = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
WORKSPACE = "cccccccc-cccc-4ccc-8ccc-cccccccccccc"
CONNECTION = "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee"
TOKEN = "synthetic-availability-control-token"
MODEL = "synthetic-model"


def eventually(predicate, timeout=2):
    until = time.monotonic() + timeout
    while time.monotonic() < until:
        if predicate(): return True
        time.sleep(.01)
    return bool(predicate())


def run_body(grants=None):
    return {"workspaceId": WORKSPACE, "message": "Synthetic availability fixture", "modelId": MODEL,
            "grantRefs": grants or [], "interfaceIds": [],
            "budgets": {"maxIterations": 8, "maxToolCalls": 12, "maxOutputTokens": 100, "maxDurationSeconds": 30}}


class AvailabilityCase(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.config = Config(Path(self.temp.name) / "state.db", LIBRARY, DEVICE,
                             hashlib.sha256(TOKEN.encode()).hexdigest(), [{"id": MODEL, "available": True}])
        self.config._verified = True
        self.store = Store(self.config.database, LIBRARY, DEVICE, self.config.models, self.config.ready)

    def tearDown(self):
        self.store.close(); self.temp.cleanup()

    def post(self, path, body): return self.store.post(path, new_id(), body)[1]

    def start_run(self, body=None, duration=2):
        self.post("/v1/runs", body or run_body())
        run = self.store.start_next()
        with self.store.transaction():
            run["_deadline"] = stamp(datetime.now(UTC) + timedelta(seconds=duration))
            self.store.put("run", run["id"], run)
        return run

    def grant(self):
        return self.post("/v1/grants", {"workspaceId": WORKSPACE, "connectionId": CONNECTION, "deviceId": DEVICE,
            "operations": [GOOGLE_TOOLS[0]], "modelId": MODEL, "dataMode": "synthetic",
            "expiresAt": stamp(datetime.now(UTC) + timedelta(hours=1))})

    def test_revoke_and_expire_after_dispatch_terminalize_without_launch(self):
        for mode in ("revoke", "expire"):
            with self.subTest(mode=mode):
                grant = self.grant()
                run = self.start_run(run_body([{"id": grant["id"], "generation": 1}]))
                if mode == "revoke":
                    self.post(f'/v1/grants/{grant["id"]}/revoke', {"generation": 1})
                else:
                    with self.store.transaction():
                        grant["expiresAt"] = stamp(datetime.now(UTC) - timedelta(seconds=1))
                        self.store.put("grant", grant["id"], grant)
                with patch("forma_runtime.supervisor.subprocess.Popen") as launch:
                    Supervisor(self.store, self.config).execute(run)
                launch.assert_not_called()
                current = self.store.get("run", run["id"])
                self.assertEqual(current["state"], "cancelled" if mode == "revoke" else "blocked")
                self.assertIn("finishedAt", current)
                # The exact same workspace/connection can be admitted again.
                replacement = self.grant()
                admitted = self.post("/v1/runs", run_body([{"id": replacement["id"], "generation": 1}]))
                self.post(f'/v1/runs/{admitted["id"]}/cancel', {})

    def test_temporary_directory_failure_terminalizes_admitted_run(self):
        run = self.start_run()
        with patch("forma_runtime.supervisor.tempfile.TemporaryDirectory", side_effect=OSError("synthetic unavailable temp")):
            Supervisor(self.store, self.config).execute(run)
        self.assertIn(self.store.get("run", run["id"])["state"], TERMINAL)
        self.post("/v1/runs", run_body())

    def test_nonreading_worker_large_start_and_tool_result_reaped(self):
        for phase in ("start", "tool-result"):
            for stop in ("deadline", "cancel", "shutdown"):
                with self.subTest(phase=phase, stop=stop):
                    body = run_body()
                    if phase == "start":
                        # The public import is valid and substantially larger than
                        # a Linux pipe; fresh workspaces permit first-import history.
                        body["workspaceId"] = new_id()
                        body["importHistory"] = [{"role": "user", "content": "x" * 12000} for _ in range(8)]
                        code = "import time; time.sleep(20)"
                    else:
                        body["workspaceId"] = new_id()
                        code = ('import sys,time; sys.stdin.readline(); '
                                'print(\'{"type":"tool","name":"forma_interface_list","args":{}}\',flush=True); '
                                'time.sleep(20)')
                    run = self.start_run(body, duration=.5 if stop == "deadline" else 10)
                    supervisor = Supervisor(self.store, self.config)
                    processes = []; errors = []; attempted_write = threading.Event()
                    original_popen = subprocess.Popen
                    original_send = supervisor.send
                    def launch(*args, **kwargs):
                        process = original_popen(*args, **kwargs); processes.append(process); return process
                    def send(*args, **kwargs):
                        # Frame shape, not transport implementation, determines
                        # when the cancellation fixture may fire.
                        frame = next((arg for arg in args if isinstance(arg, dict) and "type" in arg), None)
                        if frame and frame["type"] == phase: attempted_write.set()
                        return original_send(*args, **kwargs)
                    def execute():
                        try: supervisor.execute(run)
                        except BaseException as exc: errors.append(exc)
                    with patch("forma_runtime.supervisor.sandbox_command", return_value=[sys.executable, "-B", "-c", code]), \
                         patch("forma_runtime.supervisor.subprocess.Popen", launch), \
                         patch.object(supervisor, "send", send), \
                         patch.object(self.store, "tool", return_value={"items": ["x" * 200000]}):
                        thread = threading.Thread(target=execute, daemon=True); begin = time.monotonic(); thread.start()
                        try:
                            self.assertTrue(attempted_write.wait(2), "fixture never reached selected pipe frame")
                            self.assertTrue(eventually(lambda: processes and not select.select([], [processes[0].stdin], [], 0)[1], .25),
                                            "fixture pipe never filled; cancellation must interrupt an actual blocked write")
                            if stop == "cancel": self.post(f'/v1/runs/{run["id"]}/cancel', {})
                            elif stop == "shutdown": supervisor.stopping.set()
                            thread.join(timeout=4)
                            self.assertFalse(thread.is_alive(), "blocked pipe bypassed deadline/cancellation/reaping")
                            self.assertLess(time.monotonic() - begin, 4)
                            self.assertEqual(errors, [])
                            self.assertTrue(processes)
                            self.assertTrue(all(process.poll() is not None for process in processes))
                            self.assertEqual(self.store.get("run", run["id"])["state"],
                                             {"deadline": "blocked", "cancel": "cancelled", "shutdown": "failed"}[stop])
                        finally:
                            # Failing old-code reproduction must not leave a fixture.
                            for process in processes:
                                if process.poll() is None: process.kill(); process.wait(timeout=2)
                            thread.join(timeout=3)

    def test_upstream_header_and_body_trickle_aborted_at_run_fences(self):
        class Trickle(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_): pass
            def do_POST(self):
                self.rfile.read(int(self.headers["Content-Length"]))
                self.server.calls += 1
                self.server.started.set(); self.server.marker.touch()
                try:
                    if self.server.mode == "headers":
                        self.connection.sendall(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-Synthetic: ")
                    else:
                        self.send_response(200); self.send_header("Content-Type", "application/json")
                        self.send_header("Content-Length", "1000000"); self.end_headers()
                        self.connection.sendall(b'{"content":"')
                    while not self.server.halt.wait(.02): self.connection.sendall(b"x")
                except OSError:
                    self.server.disconnected.set()
                finally: self.server.finished.set()
        for mode in ("headers", "body"):
            for stop in ("deadline", "cancel", "shutdown", "result"):
                with self.subTest(mode=mode, stop=stop):
                    gateway = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Trickle)
                    gateway.mode = mode; gateway.calls = 0
                    gateway.started = threading.Event(); gateway.disconnected = threading.Event()
                    gateway.finished = threading.Event(); gateway.halt = threading.Event()
                    gateway.marker = Path(self.temp.name) / (mode + stop)
                    serving = threading.Thread(target=gateway.serve_forever, kwargs={"poll_interval": .02}, daemon=True); serving.start()
                    self.config.gateway_url = f"http://127.0.0.1:{gateway.server_port}/v1"
                    self.config.gateway_key = "synthetic-upstream-availability-secret"
                    self.config._gateway_address = (socket.AF_INET, ("127.0.0.1", gateway.server_port))
                    run = self.start_run(duration=1 if stop == "deadline" else 10)
                    supervisor = Supervisor(self.store, self.config)
                    processes = []; relays = []; errors = []
                    original_popen = subprocess.Popen
                    def launch(*args, **kwargs):
                        process = original_popen(*args, **kwargs); processes.append(process); return process
                    def relay(*args, **kwargs):
                        server = RelayServer(*args, **kwargs); relays.append(server); return server
                    def command(config, relay_path):
                        script = '''import http.client,json,socket,sys,threading,time,pathlib
job=json.loads(sys.stdin.readline())
class Unix(http.client.HTTPConnection):
 def connect(self):
  self.sock=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);self.sock.connect(RELAY)
def request():
 try:
  c=Unix("synthetic",timeout=15)
  c.request("POST","/v1/chat/completions",json.dumps({"model":"synthetic-model","messages":[{"role":"user","content":"Synthetic only"}]}),{"Content-Type":"application/json"})
  r=c.getresponse();r.read();c.close()
 except Exception: pass
threading.Thread(target=request,daemon=True).start()
while not pathlib.Path(MARKER).exists(): time.sleep(.01)
if RESULT:
 print(json.dumps({"type":"result","result":{"completed":True,"messages":job["history"]+[{"role":"user","content":job["message"]},{"role":"assistant","content":"Synthetic completion"}],"final_response":"Synthetic completion"}}),flush=True)
else: time.sleep(20)
'''.replace("RELAY", repr(str(relay_path))).replace("MARKER", repr(str(gateway.marker))).replace("RESULT", repr(stop == "result"))
                        return [sys.executable, "-B", "-c", script]
                    def execute():
                        try: supervisor.execute(run)
                        except BaseException as exc: errors.append(exc)
                    try:
                        with patch("forma_runtime.supervisor.sandbox_command", command), \
                             patch("forma_runtime.supervisor.subprocess.Popen", launch), \
                             patch("forma_runtime.supervisor.RelayServer", relay):
                            execution = threading.Thread(target=execute, daemon=True); execution.start()
                            try:
                                self.assertTrue(gateway.started.wait(2), "synthetic upstream was not reached")
                                begin = time.monotonic()
                                if stop == "cancel": self.post(f'/v1/runs/{run["id"]}/cancel', {})
                                elif stop == "shutdown": supervisor.stopping.set()
                                execution.join(timeout=4)
                                self.assertFalse(execution.is_alive(), "run fence did not return/reap")
                                self.assertEqual(errors, [])
                                self.assertTrue(gateway.disconnected.wait(.75), "upstream socket outlived run/reaping fence")
                                self.assertTrue(gateway.finished.is_set())
                                self.assertLess(time.monotonic() - begin, 4)
                                self.assertEqual(gateway.calls, 1)
                                self.assertTrue(all(process.poll() is not None for process in processes))
                                self.assertTrue(all(server.capacity._value == server.capacity._initial_value for server in relays),
                                                "relay handler slots remain owned after execution returned")
                                expected = {"deadline": "blocked", "cancel": "cancelled", "shutdown": "failed", "result": "succeeded"}[stop]
                                self.assertEqual(self.store.get("run", run["id"])["state"], expected)
                            finally:
                                gateway.halt.set()
                                for process in processes:
                                    if process.poll() is None: process.kill(); process.wait(timeout=2)
                                execution.join(timeout=3)
                    finally:
                        gateway.halt.set(); gateway.shutdown(); gateway.server_close(); serving.join(timeout=2)

    def test_full_valid_library_discovery_fits_metadata_budget(self):
        spec = {"schemaVersion": 1, "kind": "components", "root": {"id": "root", "type": "text", "text": "Synthetic"},
                "datasets": [{"id": "d", "columns": [{"id": "text", "type": "text"}],
                              "rows": [{"text": "x" * 2000} for _ in range(30)]}]}
        interface_spec(spec)
        ids = set()
        for _ in range(50):
            proposal = self.post("/v1/interfaces/propose", {"workspaceId": WORKSPACE, "expectedRevision": 0,
                                 "title": "\U0001f30d" * 160, "spec": spec})
            interface = self.post("/v1/interfaces/publish", {"proposalId": proposal["id"], "expectedRevision": 0})
            ids.add(interface["id"])
        run = self.start_run(duration=10)
        summaries = self.store.tool(run["id"], "forma_interface_list", {})
        self.assertEqual({item["id"] for item in summaries["items"]}, ids)
        self.assertTrue(all("spec" not in item for item in summaries["items"]))
        self.assertLess(len(canonical(summaries).encode()), 65536)
        self.assertLess(len(canonical({"type": "tool-result", "result": summaries}).encode()), MAX_FRAME)
        detail = self.store.tool(run["id"], "forma_interface_get", {"interfaceId": summaries["items"][0]["id"]})
        self.assertEqual(detail["spec"], spec)


class HeaderAvailabilityCase(unittest.TestCase):
    def make_server(self, directory, relay):
        config = Config(Path(directory) / "state.db", LIBRARY, DEVICE, hashlib.sha256(TOKEN.encode()).hexdigest(), [])
        if not relay:
            app = Application(config)
            return Server(("127.0.0.1", 0), app), app.close
        # No complete relay request is submitted, so no upstream is configured.
        server = RelayServer(str(Path(directory) / "header.sock"), RelayHandler)
        return server, lambda: None

    def connect(self, server, relay):
        if relay:
            client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); client.settimeout(2); client.connect(server.server_address)
        else: client = socket.create_connection(server.server_address, timeout=2)
        return client

    def test_idle_and_trickle_headers_release_all_capacity(self):
        for relay in (False, True):
            for trickle in (False, True):
                with self.subTest(relay=relay, trickle=trickle), tempfile.TemporaryDirectory() as directory:
                    server, cleanup = self.make_server(directory, relay)
                    server.header_timeout = .5
                    thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": .02}, daemon=True); thread.start()
                    clients = []; stop = threading.Event(); sender = None
                    try:
                        capacity = server.capacity._initial_value
                        for _ in range(capacity):
                            client = self.connect(server, relay); clients.append(client)
                            if trickle: client.sendall(b"GET /v1/capabilities HTTP/1.1\r\nX-Synthetic: ")
                            # UDS backlog exhaustion is independent of handler
                            # capacity; wait for each accept before opening N+1.
                            self.assertTrue(eventually(lambda: server.capacity._value == capacity - len(clients), .2))
                        self.assertTrue(eventually(lambda: server.capacity._value == 0, .3), "fixture did not fill connection slots")
                        # The N+1 client is rejected rather than spawning another handler.
                        extra = self.connect(server, relay)
                        try:
                            try: rejected = extra.recv(1) == b""
                            except ConnectionResetError: rejected = True
                            self.assertTrue(rejected)
                        finally: extra.close()
                        def drip():
                            while not stop.wait(.025):
                                for client in clients:
                                    try: client.sendall(b"x")
                                    except OSError: pass
                        if trickle:
                            sender = threading.Thread(target=drip, daemon=True); sender.start()
                        begin = time.monotonic()
                        self.assertTrue(eventually(lambda: server.capacity._value == capacity, 1.5),
                                        "absolute header deadline failed while unauthenticated clients held all slots")
                        self.assertLess(time.monotonic() - begin, 1.5)
                        if not relay:
                            connection = http.client.HTTPConnection(*server.server_address, timeout=2)
                            try:
                                connection.request("GET", "/v1/capabilities", headers={"Authorization": "Bearer " + TOKEN})
                                response = connection.getresponse(); response.read(); self.assertEqual(response.status, 200)
                            finally: connection.close()
                    finally:
                        stop.set()
                        if sender: sender.join(timeout=1)
                        for client in clients: client.close()
                        server.shutdown(); server.server_close(); thread.join(timeout=2); cleanup()

    def test_close_aborts_incomplete_headers_without_waiting_for_timeout(self):
        for relay in (False, True):
            with self.subTest(relay=relay), tempfile.TemporaryDirectory() as directory:
                server, cleanup = self.make_server(directory, relay)
                server.header_timeout = 30
                thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": .02}, daemon=True); thread.start()
                client = self.connect(server, relay)
                try:
                    self.assertTrue(eventually(lambda: server.capacity._value < server.capacity._initial_value))
                    begin = time.monotonic(); server.shutdown(); server.server_close()
                    self.assertTrue(eventually(lambda: server.capacity._value == server.capacity._initial_value, .5),
                                    "server_close left a pre-header handler alive")
                    self.assertLess(time.monotonic() - begin, 1)
                finally:
                    client.close(); server.shutdown(); server.server_close(); thread.join(timeout=2); cleanup()


if __name__ == "__main__": unittest.main()
