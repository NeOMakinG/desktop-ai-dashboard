"""Managed lifecycle tests: synthetic/offline, execute only on miniforum.

No Hermes import, account access or real model request. The stdio subprocess test
launches only the stdlib controller without configured models or worker resources.
"""
import copy
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import unittest
from datetime import datetime, timedelta
from pathlib import Path
from unittest.mock import patch

from forma_runtime.contracts import *
from forma_runtime.managed import Controller, model_configuration, response
from forma_runtime.supervisor import Config, Supervisor, guard_model_response
from test_runtime import BUDGETS, WORKSPACE, CONNECTION, proposal, run_body


def frame(op, **values): return dict(values, id=new_id(), op=op)


def model():
    return {"providerId": "forma-native", "gatewayUrl": "http://127.0.0.1:12345/v1", "configEpoch": new_id(),
            "gatewayKey": "synthetic-managed-private-key", "models": [{"id": "synthetic-model", "name": "Synthetic", "available": True}]}


class ManagedCase(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        root = Path(self.directory.name)
        self.bootstrap = dict(profileDir=str(root / "profile"), sourceDir=str(root / "source"),
                              pythonPath=str(root / "python" / "bin" / "python"), manifestPath=str(root / "source-proof.json"))
        self.controller = Controller()
        def verify(config):
            config._verified = bool(config.models and config.gateway_url)
            config._reason = "" if config._verified else "needs_model"
        self.patches = [patch.object(Config, "verify", verify), patch.object(Supervisor, "start"), patch.object(Supervisor, "stop")]
        for change in self.patches: change.start()

    def tearDown(self):
        self.controller.close()
        for change in reversed(self.patches): change.stop()
        self.directory.cleanup()

    def boot(self, configured=None):
        return self.controller.dispatch(frame("bootstrap", **self.bootstrap, modelConfig=configured))

    def request(self, method, path, body=None, key=None):
        fields = {"method": method, "path": path}
        if method == "POST": fields.update(body=body or {}, idempotencyKey=key or new_id())
        return self.controller.dispatch(frame("request", **fields))

    def schedule(self):
        p = self.request("POST", "/v1/interfaces/propose", proposal())["body"]
        interface = self.request("POST", "/v1/interfaces/publish", {"proposalId": p["id"], "expectedRevision": 0})["body"]
        schedule = self.request("POST", "/v1/schedules", {"workspaceId": WORKSPACE, "interfaceId": interface["id"],
            "expectedInterfaceRevision": 1, "prompt": "Synthetic refresh", "modelId": "synthetic-model", "cron": "* * * * *",
            "timezone": "UTC", "endAt": stamp(datetime.now(UTC) + timedelta(hours=1)), "maxRuns": 2, "budgets": BUDGETS, "grantRefs": []})["body"]
        return self.request("POST", "/v1/schedules/" + schedule["id"] + "/enable", {"expectedVersion": 1,
            "consent": {"scheduleVersion": 1, "modelId": "synthetic-model", "grantRefs": []}})["body"]

    def test_no_model_boot_auto_identity_private_resources_and_restart(self):
        first = self.boot()
        self.assertEqual(first["protocol"], "forma-managed-v1")
        self.assertEqual(first["capabilities"]["runtime"]["reason"], "needs_model")
        uid(first["libraryId"]); uid(first["deviceId"])
        self.assertEqual(self.request("GET", "/v1/models")["body"], {"items": []})
        self.assertEqual(self.request("POST", "/v1/runs", run_body())["status"], 422)
        self.controller.close(); self.controller = Controller()
        second = self.boot()
        self.assertEqual((first["libraryId"], first["deviceId"]), (second["libraryId"], second["deviceId"]))
        self.assertEqual(Path(self.bootstrap["profileDir"]).stat().st_mode & 0o777, 0o700)

    def test_same_epoch_restart_preserves_approved_schedule_not_no_key_execution(self):
        configured = model(); self.boot(configured)
        schedule = self.schedule()
        self.assertEqual(self.controller.dispatch(frame("lifecycle")), {"enabledSchedules": 1})
        self.controller.close(); self.controller = Controller(); self.boot()
        before = self.controller.app.store.get("schedule", schedule["id"])
        self.assertEqual(before["state"], "enabled")
        self.assertFalse(self.controller.app.config.ready())
        self.controller.dispatch(frame("configureModel", modelConfig=configured))
        after = self.controller.app.store.get("schedule", schedule["id"])
        self.assertEqual(after, before)
        renamed_catalog = copy.deepcopy(configured); renamed_catalog["models"][0]["name"] = "New display name"
        self.controller.dispatch(frame("configureModel", modelConfig=renamed_catalog))
        self.assertEqual(self.controller.app.store.get("schedule", schedule["id"]), before)

    def test_provider_epoch_changes_fence_jobs_grants_proposals_and_key_never_persists(self):
        configured = model(); self.boot(configured); schedule = self.schedule()
        store = self.controller.app.store
        grant = self.request("POST", "/v1/grants", {"workspaceId": WORKSPACE, "connectionId": CONNECTION,
            "deviceId": store.device_id, "operations": [GOOGLE_TOOLS[0]], "modelId": "synthetic-model",
            "expiresAt": stamp(datetime.now(UTC) + timedelta(hours=1)), "dataMode": "synthetic"})["body"]
        rid = new_id(); admitted = self.request("POST", "/v1/runs", run_body(grants=[{"id": grant["id"], "generation": 1}]), rid)
        self.assertEqual(admitted["status"], 202); store.start_next()
        p = store.tool(rid, "forma_interface_propose", {key: value for key, value in proposal().items() if key != "workspaceId"})
        old_config = self.controller.app.config
        changed = copy.deepcopy(configured); changed["configEpoch"] = new_id(); changed["gatewayKey"] = "synthetic-replacement-private-key"
        self.controller.dispatch(frame("configureModel", modelConfig=changed))
        self.assertEqual(store.get("run", rid)["state"], "cancelling")
        self.assertEqual(store.get("grant", grant["id"])["generation"], 2)
        self.assertEqual(store.get("schedule", schedule["id"])["state"], "paused")
        self.assertEqual(store.get("schedule", schedule["id"])["maxRuns"], 2)
        self.assertEqual(store.get("proposal", p["id"])["state"], "invalidated")
        self.assertEqual(old_config.gateway_key, "")
        for path in Path(self.bootstrap["profileDir"]).iterdir():
            if path.is_file():
                data = path.read_bytes()
                self.assertNotIn(configured["gatewayKey"].encode(), data)
                self.assertNotIn(changed["gatewayKey"].encode(), data)
        self.controller.dispatch(frame("configureModel", modelConfig=None))
        self.assertFalse(self.controller.app.config.ready())

    def test_owned_dto_is_strict_and_errors_do_not_echo_private_fields(self):
        configured = model()
        bad = frame("bootstrap", **self.bootstrap, modelConfig=dict(configured, extra="secret-marker"))
        result = decode(response(self.controller, canonical(bad).encode()))
        self.assertFalse(result["ok"])
        self.assertNotIn("secret-marker", canonical(result)); self.assertNotIn(configured["gatewayKey"], canonical(result))
        self.boot()
        for invalid in [frame("request", method="GET", path="/v1/models", body={}),
                        frame("request", method="GET", path="https://example.com/v1/models"),
                        frame("request", method="GET", path="/v1/models?x=1"),
                        frame("request", method="POST", path="/v1/runs", body={}, runtimeToken="forbidden")]:
            with self.assertRaises(Fault): self.controller.dispatch(invalid)

    def test_invalid_embedded_auth_never_appears_in_capabilities(self):
        configured = model(); configured["gatewayUrl"] = "https://:synthetic-password@synthetic.example/v1"
        with self.assertRaises(Fault): self.boot(configured)
        self.assertFalse((Path(self.bootstrap["profileDir"]) / "hermes-runtime.sqlite3").exists())
        config = Config(Path(self.directory.name) / "unused", new_id(), new_id(), "", [], gateway_url=configured["gatewayUrl"])
        self.assertEqual(config.model_origin, "")

    def test_optional_auth_and_empty_secret_guard(self):
        config = model(); config.pop("gatewayKey")
        self.assertEqual(model_configuration(config)["gatewayKey"], "")
        self.boot(config)
        self.assertTrue(self.controller.app.config.ready())
        guard_model_response(b'{"choices":[]}', "application/json", "")
        with self.assertRaises(Fault): guard_model_response(b'bad json', "application/json", "")


class ManagedProcessCase(unittest.TestCase):
    def test_mac_profile_explicitly_denies_sibling_process_info(self):
        from forma_runtime.platforms import mac_profile
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            config = Config(root / "db", new_id(), new_id(), "", [], source=root / "source", python=root / "python" / "bin" / "python3.12")
            profile = mac_profile(config, root / "relay.sock")
            self.assertIn("(deny process-info*)", profile)
            self.assertIn("(allow process-info* (target self))", profile)
            self.assertNotIn("(allow sysctl-read)", profile)
            self.assertIn('(sysctl-name "kern.hostname")', profile)
            self.assertNotIn('(sysctl-name "kern.procargs2")', profile)

    def test_private_stdio_eof_releases_identity_lock_without_worker(self):
        root = Path(__file__).resolve().parents[1]
        with tempfile.TemporaryDirectory() as directory:
            boot = frame("bootstrap", profileDir=directory, sourceDir=directory + "/absent",
                         pythonPath=sys.executable, manifestPath=directory + "/absent-manifest")
            command = [sys.executable, "-I", "-B", str(root / "forma_runtime" / "managed.py")]
            outputs = []
            for _ in range(2):
                result = subprocess.run(command, input=canonical(boot) + "\n", capture_output=True, text=True, timeout=10,
                    env={"PATH": "/usr/bin:/bin", "FORMA_MODEL_GATEWAY_KEY": "synthetic-do-not-read-env"})
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stderr, "")
                self.assertNotIn("synthetic-do-not-read-env", result.stdout)
                answer = decode(result.stdout); self.assertTrue(answer["ok"])
                self.assertEqual(answer["result"]["capabilities"]["runtime"]["reason"], "needs_model")
                outputs.append(answer["result"])
            self.assertEqual(outputs[0]["libraryId"], outputs[1]["libraryId"])

    def test_provider_freeze_before_dns_and_managed_public_https_policy(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory); source = root / "source"; source.mkdir()
            (source / "run_agent.py").write_text("# synthetic source, never imported\n")
            manifest = root / "manifest.json"
            manifest.write_text(canonical({"revision": REVISION, "files": [{"path": "run_agent.py", "sha256": hashlib.sha256((source / "run_agent.py").read_bytes()).hexdigest()}]}))
            config = Config(root / "db", new_id(), new_id(), "", [{"id": "synthetic-model"}], source=source,
                python=Path(sys.executable), source_manifest=manifest, managed=True,
                gateway_url="https://higgsfield.ai/v1", gateway_key="")
            with patch("forma_runtime.supervisor.sys.platform", "linux"), patch("forma_runtime.supervisor.shutil.which", return_value="/usr/bin/bwrap"), patch("forma_runtime.supervisor.socket.getaddrinfo") as dns:
                with self.assertRaises(Fault) as failure: config.verify()
                self.assertEqual(failure.exception.code, "provider_frozen"); dns.assert_not_called()
                config.gateway_url = "https://synthetic-public.example/v1"
                dns.return_value = [(2, 1, 6, "", ("93.184.216.34", 443))]
                config.verify(); self.assertTrue(config.ready())
                config.managed = False; config.gateway_key = "synthetic-only"
                with self.assertRaises(Fault): config.verify()
                config.managed = True; config.gateway_url = "http://synthetic-public.example/v1"
                with self.assertRaises(Fault): config.verify()


if __name__ == "__main__": unittest.main()
