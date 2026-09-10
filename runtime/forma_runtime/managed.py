"""App-owned private stdio control plane; never listens on TCP or reads credentials.

Launch with bundled Python -I -B /owned/runtime/forma_runtime/managed.py.
Only a native parent holding both anonymous pipes may bootstrap this process.
"""
from __future__ import annotations

import os
import select
import signal
import sys
import time
from pathlib import Path
from urllib.parse import urlsplit

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from forma_runtime.contracts import *
from forma_runtime.server import Application
from forma_runtime.supervisor import Config, Supervisor

PROTOCOL = "forma-managed-v1"
MAX_REQUEST = 600_000
MAX_RESPONSE = 400_000


def owned_path(value):
    text(value, 4096, 1)
    path = Path(value)
    require(path.is_absolute() and not any(ord(c) < 32 for c in value), "Expected owned absolute path")
    return path


def model_configuration(value):
    if value is None: return {"providerId": "", "gatewayUrl": "", "gatewayKey": "", "models": [], "dailyLimit": 192}
    obj(value, ("providerId", "gatewayUrl", "models"), ("gatewayKey", "dailyLimit", "configEpoch"))
    text(value["providerId"], 200, 1); text(value["gatewayUrl"], 2048, 1); text(value.get("gatewayKey", ""), 8192)
    require(not any(c in value.get("gatewayKey", "") for c in "\r\n\t"), "Invalid gateway credential")
    if "configEpoch" in value: uid(value["configEpoch"])
    # Validate BEFORE a nonsecret provider descriptor can reach SQLite or DTOs.
    # Native already rejects userinfo, but the private control boundary does not
    # turn a malformed URL containing credentials into safe metadata.
    try:
        url = urlsplit(value["gatewayUrl"])
        require(url.scheme in ("http", "https") and bool(url.hostname) and not url.username and not url.password
                and not url.query and not url.fragment and url.path.rstrip("/") == "/v1", "Invalid model provider URL")
        if url.port is not None: integer(url.port, 1, 65535)
        require("higgsfield" not in url.hostname.lower(), "Provider account access is frozen", "provider_frozen", 403)
    except ValueError as exc: raise Fault("invalid_request", "Invalid model provider URL") from exc
    ids = set()
    for model in array(value["models"], 200):
        obj(model, ("id", "name", "available"), ("reason",))
        text(model["id"], 200, 1); text(model["name"], 200, 1)
        require(type(model["available"]) is bool and model["id"] not in ids, "Invalid model catalog")
        if "reason" in model: text(model["reason"], 200, 1)
        ids.add(model["id"])
    integer(value.get("dailyLimit", 192), 1, 192)
    return dict(value, gatewayKey=value.get("gatewayKey", ""), dailyLimit=value.get("dailyLimit", 192))


def model_scope(model):
    # Native rotates configEpoch for credential/provider changes. Catalog display
    # refresh and workspace model selection do not transfer provider authority.
    return {key: model.get(key) for key in ("providerId", "gatewayUrl", "configEpoch")}


class Controller:
    def __init__(self):
        self.app = None
        self.paths = None
        self.model = None
        self.shutdown = False

    def authorize_configuration(self, model, force=False):
        scope = model_scope(model)
        with self.app.store.lock:
            previous = self.app.store.get("meta", "model-scope", optional=True)
        if force or not scope.get("configEpoch") or previous != scope:
            self.app.store.fence_model_configuration()
        with self.app.store.transaction(): self.app.store.put("meta", "model-scope", scope)

    def lifecycle(self):
        from datetime import datetime
        with self.app.store.lock:
            count = sum(schedule["state"] == "enabled" and schedule["runsStarted"] < schedule["maxRuns"]
                        and instant(schedule["endAt"]) > datetime.now(UTC) for schedule in self.app.store.all("schedule"))
        return {"enabledSchedules": count}

    def descriptor(self):
        return {"protocol": PROTOCOL, "libraryId": self.app.config.library_id,
                "deviceId": self.app.config.device_id, "capabilities": self.app.capabilities(), **self.lifecycle()}

    def configuration(self, model):
        config = Config(database=self.paths["profileDir"] / "hermes-runtime.sqlite3",
            library_id=self.app.config.library_id if self.app else None,
            device_id=self.app.config.device_id if self.app else None,
            token_sha256="", models=model["models"], source=self.paths["sourceDir"],
            python=self.paths["pythonPath"], source_manifest=self.paths["manifestPath"],
            gateway_url=model["gatewayUrl"].rstrip("/"), gateway_key=model["gatewayKey"],
            daily_limit=model["dailyLimit"], managed=True)
        try: config.verify()
        except Exception:
            config._verified = False; config._reason = "configuration_unverified"
        return config

    def dispatch(self, frame):
        require(isinstance(frame, dict), "Expected control frame")
        uid(frame.get("id")); op = frame.get("op")
        if op == "bootstrap":
            obj(frame, ("id", "op", "profileDir", "sourceDir", "pythonPath", "manifestPath"), ("modelConfig",))
            require(self.app is None, "Controller already bootstrapped", "already_bootstrapped", 409)
            self.paths = {key: owned_path(frame[key]) for key in ("profileDir", "sourceDir", "pythonPath", "manifestPath")}
            profile = self.paths["profileDir"]
            profile.mkdir(parents=True, exist_ok=True, mode=0o700)
            require(not profile.is_symlink() and profile.is_dir() and profile.stat().st_uid == os.getuid(),
                    "Invalid native profile directory", "profile_unavailable", 503)
            # The native host supplies its dedicated runtime profile directory,
            # not the whole app library. It contains no gateway credential file.
            os.chmod(profile, 0o700)
            database = profile / "hermes-runtime.sqlite3"
            require(not database.is_symlink(), "Database symlink denied", "profile_unavailable", 503)
            model = model_configuration(frame.get("modelConfig"))
            config = self.configuration(model)
            self.app = Application(config)
            # No credential or credential-derived hash is persisted. Native's
            # durable nonsecret epoch proves unchanged prior provider approval.
            if frame.get("modelConfig") is not None: self.authorize_configuration(model)
            self.model = model
            self.app.supervisor.start()
            return self.descriptor()
        require(self.app is not None and not self.shutdown, "Controller not running", "not_bootstrapped", 409)
        if op == "lifecycle":
            obj(frame, ("id", "op"))
            return self.lifecycle()
        if op == "configureModel":
            obj(frame, ("id", "op", "modelConfig"))
            model = model_configuration(frame["modelConfig"])
            if model == self.model and (self.app.config.ready() or not model["gatewayUrl"]): return self.descriptor()
            same_authority = (self.model is not None and bool(self.model["gatewayUrl"])
                              and model_scope(model) == model_scope(self.model)
                              and model["gatewayKey"] == self.model["gatewayKey"])
            if same_authority and self.app.config.ready():
                # Catalog refresh is not a provider change. The store independently
                # fences a run if its selected model is removed/unavailable.
                with self.app.store.lock:
                    self.app.config.models = self.app.store.models = model["models"]
                    self.app.config.daily_limit = model["dailyLimit"]
                self.model = model
                return self.descriptor()
            # Fence BEFORE validation/DNS/launch and never replace a relay's
            # configuration while an old socket/worker remains live.
            self.app.config._verified = False
            self.authorize_configuration(model, force=bool(self.model and self.model["gatewayUrl"] and not same_authority))
            self.app.supervisor.stop()
            self.app.config.gateway_key = ""
            self.model = None
            config = self.configuration(model)
            self.app.config = config
            self.app.store.models, self.app.store.ready = config.models, config.ready
            self.app.supervisor = Supervisor(self.app.store, config)
            self.model = model
            self.app.supervisor.start()
            return self.descriptor()
        if op == "request":
            obj(frame, ("id", "op", "method", "path"), ("query", "idempotencyKey", "body"))
            require(frame["method"] in ("GET", "POST"), "Method not allowed")
            path = text(frame["path"], 2048, 1)
            require(path.startswith("/v1/") and not any(c in path for c in "?#%\\\r\n"), "Invalid route")
            query = frame.get("query", {})
            require(isinstance(query, dict) and len(query) <= 8, "Invalid query")
            for key, value in query.items(): text(key, 100, 1); text(value, 2048)
            headers = {"Content-Type": "application/json"}
            if "idempotencyKey" in frame: headers["Idempotency-Key"] = uid(frame["idempotencyKey"])
            if frame["method"] == "GET": require("body" not in frame and "idempotencyKey" not in frame, "GET has no mutation body")
            raw = canonical(frame.get("body", {})).encode()
            require(len(raw) <= 524288, "Request body exceeds bound")
            try:
                status, body = self.app.dispatch_owned(frame["method"], path, query, headers, raw)
            except Fault as exc: status, body = exc.status, exc.wire()
            require(len(canonical(body).encode()) <= 393216, "Response exceeds protocol bound", "response_limit", 500)
            return {"status": status, "body": body}
        if op == "shutdown":
            obj(frame, ("id", "op"))
            self.close()
            return {"stopped": True}
        raise Fault("invalid_request", "Unknown control operation")

    def close(self):
        self.shutdown = True
        if self.app:
            app, self.app = self.app, None
            app.config._verified = False
            app.close()
            app.config.gateway_key = ""
        self.model = None


def response(controller, raw):
    identity = None
    try:
        frame = decode(raw)
        if isinstance(frame, dict): identity = uid(frame.get("id"))
        result = controller.dispatch(frame)
        value = {"id": identity, "ok": True, "result": result}
    except Fault as exc:
        value = {"id": identity, "ok": False, "error": exc.wire()["error"]}
    except Exception:
        value = {"id": identity, "ok": False, "error": Fault("control_failed", "Managed runtime operation failed", 503).wire()["error"]}
    payload = canonical(value).encode() + b"\n"
    require(len(payload) <= MAX_RESPONSE, "Control response exceeds bound", "response_limit", 500)
    return payload


def main():
    # Unsupported platforms fail explicitly before importing Hermes or launching
    # anything. macOS/Linux parent containment adapters are independently gated.
    require(sys.platform in ("darwin", "linux"), "Managed worker containment unavailable", "platform_unavailable", 503)
    os.umask(0o077)
    controller = Controller()
    stopping = False
    def stop(*_):
        nonlocal stopping
        stopping = True
    signal.signal(signal.SIGTERM, stop); signal.signal(signal.SIGINT, stop)
    source, sink = sys.stdin.fileno(), sys.stdout.fileno()
    os.set_blocking(source, False); os.set_blocking(sink, False)
    pending = bytearray(); partial_deadline = None
    try:
        while not stopping and not controller.shutdown:
            readable, _, _ = select.select([source], [], [], .1)
            if partial_deadline is not None and time.monotonic() > partial_deadline: break
            if not readable: continue
            chunk = os.read(source, 65536)
            if not chunk: break
            pending.extend(chunk)
            if partial_deadline is None: partial_deadline = time.monotonic() + 15
            while b"\n" in pending and not stopping and not controller.shutdown:
                end = pending.index(b"\n")
                require(end + 1 <= MAX_REQUEST, "Control frame exceeds bound")
                raw = bytes(pending[:end]); del pending[:end + 1]
                output = memoryview(response(controller, raw)); deadline = time.monotonic() + 15
                while output and not stopping:
                    require(time.monotonic() < deadline, "Control response timed out")
                    _, writable, _ = select.select([], [sink], [], .1)
                    if writable:
                        count = os.write(sink, output[:65536]); require(count > 0, "Control pipe closed")
                        output = output[count:]
                partial_deadline = time.monotonic() + 15 if pending else None
            require(len(pending) < MAX_REQUEST, "Control frame exceeds bound")
    finally:
        controller.close()


if __name__ == "__main__":
    try: main()
    except Exception:
        # A fatal error tears down the owned process. No exception/config/secret
        # text is printed on stdout or stderr, including during bootstrap.
        sys.exit(1)
