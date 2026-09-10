"""Opt-in Linux synthetic isolation/relay tests; never import or call Hermes.

Set FORMA_SANDBOX_TEST_SOURCE/PYTHON/MANIFEST to the dedicated miniforum install.
All upstream responses below are explicitly synthetic loopback fixtures.
"""
import hashlib
import http.server
import json
import os
import socketserver
import subprocess
import tempfile
import threading
import unittest
from pathlib import Path

from forma_runtime.contracts import *
from forma_runtime.store import Store
from forma_runtime.supervisor import Config, RelayHandler, RelayServer, sandbox_command

LIBRARY = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa"
DEVICE = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb"
WORKSPACE = "cccccccc-cccc-4ccc-8ccc-cccccccccccc"


class Fixture(http.server.BaseHTTPRequestHandler):
    def log_message(self, *_): pass
    def do_POST(self):
        self.server.calls += 1
        body = self.rfile.read(int(self.headers["Content-Length"]))
        self.server.requests.append(json.loads(body))
        payload = json.loads(body)
        mode = getattr(self.server, "mode", "plain")
        usage = {"prompt_tokens": 100, "completion_tokens": 50, "total_tokens": 150}
        message = {"role": "assistant", "content": "Synthetic transport fixture"}; finish = "stop"
        if mode.startswith("hermes") and self.server.calls % 2 == 1:
            if mode == "hermes_propose":
                name = "forma_interface_propose"
                args = {"expectedRevision": 0, "title": "Hermes synthetic fixture", "spec": {"schemaVersion": 1, "kind": "components",
                    "root": {"id": "root", "type": "text", "text": "Generated through actual Hermes over a deterministic fixture model."}, "datasets": []}}
            else:
                name = "terminal"; args = {"command": "printf forbidden-synthetic-tool"}
            message = {"role": "assistant", "content": None, "tool_calls": [{"id": "call_synthetic_1", "type": "function", "function": {"name": name, "arguments": json.dumps(args)}}]}
            finish = "tool_calls"
        base = {"id": "synthetic", "created": 1, "model": "synthetic-model"}
        if payload.get("stream"):
            delta = dict(message)
            if "tool_calls" in delta:
                delta["tool_calls"] = [dict(call, index=i) for i, call in enumerate(delta["tool_calls"])]
            chunks = [dict(base, object="chat.completion.chunk", choices=[{"index": 0, "delta": delta, "finish_reason": None}]),
                      dict(base, object="chat.completion.chunk", choices=[{"index": 0, "delta": {}, "finish_reason": finish}], usage=usage)]
            data = ("".join("data: " + json.dumps(chunk) + "\n\n" for chunk in chunks) + "data: [DONE]\n\n").encode()
            content_type = "text/event-stream"
        else:
            data = json.dumps(dict(base, object="chat.completion", choices=[{"index": 0, "message": message, "finish_reason": finish}], usage=usage)).encode()
            content_type = "application/json"
        self.send_response(200); self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(data))); self.end_headers(); self.wfile.write(data)


@unittest.skipUnless(os.getenv("FORMA_SANDBOX_TEST_SOURCE"), "dedicated miniforum opt-in required")
class SandboxCase(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.gateway = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Fixture)
        self.gateway.calls = 0; self.gateway.requests = []
        threading.Thread(target=self.gateway.serve_forever, daemon=True).start()
        self.config = Config(Path(self.temp.name) / "state.db", LIBRARY, DEVICE, "0" * 64,
            [{"id": "synthetic-model", "available": True}],
            source=Path(os.environ["FORMA_SANDBOX_TEST_SOURCE"]), python=Path(os.environ["FORMA_SANDBOX_TEST_PYTHON"]),
            source_manifest=Path(os.environ["FORMA_SANDBOX_TEST_MANIFEST"]),
            gateway_url=f"http://127.0.0.1:{self.gateway.server_port}/v1", gateway_key="synthetic-parent-only-upstream-secret", daily_limit=2)
        self.config.verify()
        self.store = Store(self.config.database, LIBRARY, DEVICE, self.config.models, self.config.ready)
        body = {"workspaceId": WORKSPACE, "message": "Synthetic", "modelId": "synthetic-model", "grantRefs": [], "interfaceIds": [],
                "budgets": {"maxIterations": 8, "maxToolCalls": 12, "maxOutputTokens": 100, "maxDurationSeconds": 180}}
        self.run = self.store.post("/v1/runs", new_id(), body)[1]; self.store.start_next()
        self.relay_path = Path(self.temp.name) / "relay.sock"
        self.relay = RelayServer(str(self.relay_path), RelayHandler)
        self.relay.store, self.relay.config, self.relay.run_id = self.store, self.config, self.run["id"]
        self.relay.model_id, self.relay.allowed_tools = "synthetic-model", set(TOOLS)
        threading.Thread(target=self.relay.serve_forever, daemon=True).start()

    def tearDown(self):
        self.relay.shutdown(); self.relay.server_close()
        self.gateway.shutdown(); self.gateway.server_close()
        self.store.close(); self.temp.cleanup()

    def script(self, code):
        command = sandbox_command(self.config, self.relay_path)
        # Same production mounts/environment/namespace boundary, substituting a
        # stdlib-only fixture script for the real Hermes entrypoint.
        command[-2:] = ["-c", code]
        result = subprocess.run(command, capture_output=True, text=True, timeout=15,
                                env={"PATH": "/usr/bin:/bin", "DO_NOT_LEAK": "synthetic-host-secret"})
        self.assertEqual(result.returncode, 0, result.stderr[:2000])
        return json.loads(result.stdout)

    def test_no_network_host_home_secrets_or_source_writes(self):
        result = self.script(r'''
import json,os,socket,pathlib
out={"home":os.environ.get("HOME"),"safe":os.environ.get("HERMES_SAFE_MODE"),"hostSecret":os.environ.get("DO_NOT_LEAK"),"hostHome":pathlib.Path("/home/neo").exists()}
s=socket.socket();s.settimeout(.2)
try:s.connect(("1.1.1.1",443));out["network"]=True
except OSError:out["network"]=False
try:(pathlib.Path(os.environ["FORMA_HERMES_SOURCE"])/"synthetic-denied-write").write_text("x");out["sourceWritable"]=True
except OSError:out["sourceWritable"]=False
pathlib.Path("/home/forma/synthetic").write_text("allowed")
out["profileWritable"]=True
out["status"]={line.split(":")[0]:line.split(":")[1].strip() for line in pathlib.Path("/proc/self/status").read_text().splitlines() if line.startswith(("CapEff:","NoNewPrivs:"))}
print(json.dumps(out))
''')
        self.assertEqual(result["home"], "/home/forma"); self.assertEqual(result["safe"], "1")
        self.assertIsNone(result["hostSecret"]); self.assertFalse(result["hostHome"])
        self.assertFalse(result["network"]); self.assertFalse(result["sourceWritable"])
        self.assertEqual(result["status"]["CapEff"], "0000000000000000")
        self.assertEqual(result["status"]["NoNewPrivs"], "1")
        self.assertEqual(self.gateway.calls, 0)

    @unittest.skipUnless(os.getenv("FORMA_HERMES_FIXTURE_TESTS") == "1", "actual Hermes fixture import needs explicit opt-in")
    def test_actual_hermes_tool_loop_and_forged_builtin_denial(self):
        from unittest.mock import patch
        from forma_runtime.supervisor import Supervisor
        original_command, original_popen = sandbox_command, subprocess.Popen
        self.config.daily_limit = 8
        for mode in ("hermes_propose", "hermes_forbidden"):
            if self.store.get("run", self.run["id"])["state"] in TERMINAL:
                body = {"workspaceId": WORKSPACE, "message": "Synthetic tool validation", "modelId": "synthetic-model", "grantRefs": [], "interfaceIds": [],
                        "budgets": {"maxIterations": 8, "maxToolCalls": 12, "maxOutputTokens": 4096, "maxDurationSeconds": 60}}
                self.run = self.store.post("/v1/runs", new_id(), body)[1]; self.store.start_next()
            self.gateway.mode = mode; self.gateway.calls = 0; self.gateway.requests = []
            def command(config, relay_path):
                result = original_command(config, relay_path)
                result[-4:-4] = ["--setenv", "FORMA_WORKER_DIAGNOSTICS", "synthetic-only"]
                return result
            with tempfile.TemporaryFile() as diagnostics:
                def launch(*args, **kwargs):
                    kwargs["stderr"] = diagnostics
                    return original_popen(*args, **kwargs)
                with patch("forma_runtime.supervisor.sandbox_command", command), patch("forma_runtime.supervisor.subprocess.Popen", launch):
                    Supervisor(self.store, self.config).execute(self.store.get("run", self.run["id"]))
                diagnostics.seek(0); diagnostic_text = diagnostics.read(12000).decode(errors="replace")
            actual = self.store.get("run", self.run["id"])
            errors = [x["payload"] for x in self.store.get_route('/v1/runs/' + self.run["id"], {})["events"] if x["type"] == "error"]
            self.assertEqual(actual["state"], "succeeded", f'{actual.get("reason")} calls={self.gateway.calls} errors={errors}\n{diagnostic_text}')
            self.assertEqual(self.gateway.calls, 2)
            names = {tool["function"]["name"] for tool in self.gateway.requests[0]["tools"]}
            self.assertEqual(names, set(TOOLS) - set(GOOGLE_TOOLS))
            self.assertNotIn("terminal", names)
            proposals = [p for p in self.store.all("proposal") if p.get("sourceRunId") == self.run["id"]]
            if mode == "hermes_propose": self.assertEqual(len(proposals), 1)
            else:
                self.assertEqual(proposals, [])
                self.assertEqual(actual["_toolCalls"], 0)
                self.assertIn("terminal", json.dumps(self.gateway.requests[1]["messages"]))

    def test_supervisor_reaps_deadline_and_oversized_frame(self):
        from unittest.mock import patch
        from datetime import datetime, timedelta
        from forma_runtime.supervisor import Supervisor
        original = sandbox_command
        for code, expected in [("import time;time.sleep(8)", "blocked"),
                               ("import sys;sys.stdout.write('x'*2300000+'\\n');sys.stdout.flush()", "failed")]:
            if self.store.get("run", self.run["id"])["state"] in TERMINAL:
                body = {"workspaceId": WORKSPACE, "message": "Synthetic second", "modelId": "synthetic-model", "grantRefs": [], "interfaceIds": [],
                        "budgets": {"maxIterations": 8, "maxToolCalls": 12, "maxOutputTokens": 100, "maxDurationSeconds": 180}}
                self.run = self.store.post("/v1/runs", new_id(), body)[1]; self.store.start_next()
            with self.store.transaction():
                run = self.store.get("run", self.run["id"])
                run["_deadline"] = stamp(datetime.now(UTC) + timedelta(seconds=1 if expected == "blocked" else 5))
                self.store.put("run", run["id"], run)
            def command(config, relay_path):
                result = original(config, relay_path); result[-2:] = ["-c", code]; return result
            with patch("forma_runtime.supervisor.sandbox_command", command):
                Supervisor(self.store, self.config).execute(run)
            self.assertEqual(self.store.get("run", run["id"])["state"], expected)
        self.assertEqual(self.gateway.calls, 0)

    def test_unix_relay_exact_model_route_and_daily_ceiling(self):
        result = self.script(r'''
import socket,json,http.client
class Unix(http.client.HTTPConnection):
 def connect(self):
  self.sock=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);self.sock.connect("/run/forma-relay.sock")
results=[]
for path,model in [("/v1/models","synthetic-model"),("/v1/chat/completions","wrong-model"),("/v1/chat/completions","synthetic-model"),("/v1/chat/completions","synthetic-model"),("/v1/chat/completions","synthetic-model")]:
 c=Unix("forma-gateway",timeout=5);body=json.dumps({"model":model,"messages":[{"role":"user","content":"Synthetic only"}],"max_tokens":10})
 c.request("POST",path,body,{"Content-Type":"application/json"});r=c.getresponse();payload=r.read();results.append({"status":r.status,"payload":payload.decode()});c.close()
print(json.dumps(results))
''')
        self.assertEqual([x["status"] for x in result], [403, 403, 200, 200, 429])
        self.assertEqual(self.gateway.calls, 2)
        self.assertNotIn(self.config.gateway_key, json.dumps(result))
        self.assertTrue(all(x["model"] == "synthetic-model" for x in self.gateway.requests))


if __name__ == "__main__": unittest.main()
