"""Single-worker Linux supervisor and narrow, budgeted Unix model relay."""
from __future__ import annotations

import hashlib
import http.server
import http.client
import ipaddress
import json
import os
import select
import shutil
import signal
import socket
import ssl
import socketserver
import subprocess
import sys
import tempfile
import threading
import time
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path
from urllib.parse import urlsplit

from .contracts import *

MAX_FRAME = 2_200_000
MAX_PROVIDER_BYTES = 8_000_000


@dataclass
class Config:
    database: Path
    library_id: str
    device_id: str
    token_sha256: str
    models: list
    source: Path | None = None
    python: Path | None = None
    gateway_url: str = ""
    gateway_key: str = field(default="", repr=False)
    source_manifest: Path | None = None
    daily_limit: int = 192
    bind: str = "127.0.0.1"
    port: int = 9473
    managed: bool = False
    _verified: bool = False
    _reason: str = "not_configured"
    _gateway_address: tuple | None = None

    @classmethod
    def from_env(cls):
        env = os.environ
        path = lambda name: Path(env[name]).absolute() if env.get(name) else None
        models = [{"id": text(x.strip(), 200, 1), "name": x.strip(), "available": True}
                  for x in env.get("FORMA_RUNTIME_MODELS", "").split(",") if x.strip()]
        value = cls(database=Path(env["FORMA_RUNTIME_DB"]).resolve(), library_id=uid(env["FORMA_RUNTIME_LIBRARY_ID"]),
            device_id=uid(env["FORMA_RUNTIME_DEVICE_ID"]), token_sha256=env["FORMA_RUNTIME_TOKEN_SHA256"], models=models,
            source=path("FORMA_HERMES_SOURCE"), python=path("FORMA_HERMES_PYTHON"),
            source_manifest=path("FORMA_HERMES_MANIFEST"), gateway_url=env.get("FORMA_MODEL_GATEWAY_URL", "").rstrip("/"),
            gateway_key=env.get("FORMA_MODEL_GATEWAY_KEY", ""), daily_limit=int(env.get("FORMA_DAILY_MODEL_REQUESTS", "192")),
            bind=env.get("FORMA_RUNTIME_BIND", "127.0.0.1"), port=int(env.get("FORMA_RUNTIME_PORT", "9473")))
        require(re.fullmatch(r"[0-9a-f]{64}", value.token_sha256), "Expected dedicated runtime token SHA256")
        integer(value.daily_limit, 1, 192); integer(value.port, 1024, 65535)
        # TLS belongs to an explicitly provisioned proxy; never expose plaintext
        # bearer transport on a general interface by accident.
        require(value.bind in ("127.0.0.1", "::1"), "Bind only to loopback behind authenticated encrypted transport")
        return value

    @property
    def model_origin(self):
        url = urlsplit(self.gateway_url)
        return f"{url.scheme}://{url.netloc}" if url.scheme and url.netloc and not url.username and not url.password else ""

    def ready(self):
        return self._verified

    def verify(self):
        """Runs only on explicit service startup, not module import."""
        self._verified = False
        if sys.platform not in ("linux", "darwin"):
            self._reason = "platform_sandbox_unavailable"; return
        if not self.models or not self.gateway_url or (not self.managed and not self.gateway_key):
            self._reason = "needs_model" if self.managed else "not_configured"; return
        if not self.source or not self.python or not self.source_manifest:
            self._reason = "runtime_resources_missing"; return
        if sys.platform == "linux" and not shutil.which("bwrap"):
            self._reason = "bubblewrap_unavailable"; return
        if sys.platform == "darwin" and not Path("/usr/bin/sandbox-exec").is_file():
            self._reason = "sandbox_exec_unavailable"; return
        url = urlsplit(self.gateway_url)
        require(url.scheme in ("http", "https") and not url.username and not url.password and not url.query
                and not url.fragment and url.path == "/v1", "Gateway must be explicit /v1 endpoint without credentials")
        host = url.hostname or ""
        require("higgsfield" not in host.lower(), "Provider account access is frozen", "provider_frozen", 403)
        try:
            address = ipaddress.ip_address(host)
            private = address.is_private or address in ipaddress.ip_network("100.64.0.0/10") if address.version == 4 else address.is_private
        except ValueError:
            private = host in ("bosgame", "miniforum", "localhost") or host.endswith(".ts.net")
        require(private or (self.managed and url.scheme == "https"),
                "Public model providers require explicit native-owned HTTPS configuration")
        # Resolve only at explicit startup, not inside deadline-bound request
        # threads. Connections use this numeric address; TLS retains the hostname.
        addresses = socket.getaddrinfo(host, url.port or (443 if url.scheme == "https" else 80), type=socket.SOCK_STREAM)
        addresses.sort(key=lambda entry: entry[0] != socket.AF_INET)
        require(bool(addresses), "Gateway address unavailable")
        family, _, _, _, address = addresses[0]
        resolved = ipaddress.ip_address(address[0])
        require(resolved.is_private or (resolved.version == 4 and resolved in ipaddress.ip_network("100.64.0.0/10"))
                or (self.managed and url.scheme == "https"), "Gateway resolved outside approved address space")
        self._gateway_address = (family, address)
        require(self.python.is_file() and (self.source / "run_agent.py").is_file(), "Dedicated Hermes install missing")
        manifest_bytes = self.source_manifest.read_bytes()
        manifest = decode(manifest_bytes)
        # Accept the tracked-export receipt produced by the dedicated installer,
        # not a profile copy or a moving branch/version string.
        if isinstance(manifest, list):
            export = decode(self.source_manifest.with_name("source-export.json").read_bytes())
            require(export.get("commit") == REVISION and export.get("manifest_sha256") == hashlib.sha256(manifest_bytes).hexdigest(),
                    "Source export receipt does not match pin/manifest")
            manifest = {"revision": export["commit"], "files": manifest}
        require(isinstance(manifest, dict) and manifest.get("revision") == REVISION and isinstance(manifest.get("files"), list),
                "Expected pinned source manifest")
        expected_files = {entry["path"] for entry in manifest["files"]}
        actual_files = set()
        for item in self.source.rglob("*"):
            require(not item.is_symlink(), "Symlink in source tree")
            if item.is_file(): actual_files.add(item.relative_to(self.source).as_posix())
        require(actual_files == expected_files, "Unexpected or missing pinned source files")
        for entry in manifest["files"]:
            relative = Path(entry["path"])
            require(not relative.is_absolute() and ".." not in relative.parts, "Invalid manifest path")
            file = self.source / relative
            require(file.is_file() and not file.is_symlink() and hashlib.sha256(file.read_bytes()).hexdigest() == entry["sha256"],
                    "Pinned source integrity mismatch")
        require(any(x["path"] == "run_agent.py" for x in manifest["files"]), "Manifest lacks Hermes entrypoint")
        if sys.platform == "darwin":
            from .platforms import mac_probe
            mac_probe(self)
        self._verified = True; self._reason = ""


def guard_model_response(data, content_type, secret):
    """Bounded raw/decoded/one-escape reflection guard, including SDK SSE joins.

    This is not general encoded-secret DLP. The key stays parent-only even when
    a hostile fixture/provider JSON-escapes it or splits text/tool arguments.
    """
    require(len(data) <= MAX_PROVIDER_BYTES, "Model response too large", "model_response_too_large", 502)
    try: raw = data.decode("utf-8")
    except UnicodeError as exc: raise Fault("model_response_rejected", "Invalid model response encoding", 502) from exc
    escape = re.compile(r'\\(?:["\\/bfnrt]|u[0-9a-fA-F]{4})')
    def check(value):
        require(not secret or secret not in value, "Model response rejected", "model_response_rejected", 502)
        try:
            decoded = escape.sub(lambda m: json.loads('"' + m.group(0) + '"'), value)
        except (ValueError, UnicodeError):
            decoded = value
        require(not secret or secret not in decoded, "Model response rejected", "model_response_rejected", 502)
    check(raw)
    if content_type == "application/json": documents = [decode(raw)]
    else:
        documents = []; event_lines = []
        # SSE recognizes LF, CRLF and bare CR equally, including comments.
        for line in raw.replace("\r\n", "\n").replace("\r", "\n").split("\n") + [""]:
            if line.startswith("data:"): event_lines.append(line[5:].lstrip(" "))
            elif line == "" and event_lines:
                payload = "\n".join(event_lines); event_lines = []
                if payload != "[DONE]": documents.append(decode(payload))
        require(len(documents) <= 100000, "Too many model stream events", "model_response_rejected", 502)
    streams = {}; nodes = 0
    for document in documents:
        pending = [(document, 0)]
        while pending:
            value, depth = pending.pop(); nodes += 1
            require(nodes <= 1000000 and depth <= 64, "Model response structure too large", "model_response_rejected", 502)
            if isinstance(value, str): check(value)
            elif isinstance(value, dict):
                for key, item in value.items(): check(key); pending.append((item, depth + 1))
            elif isinstance(value, list): pending.extend((item, depth + 1) for item in value)
        if content_type != "text/event-stream" or not isinstance(document, dict): continue
        for choice in document.get("choices", []):
            require(isinstance(choice, dict), "Invalid stream choice", "model_response_rejected", 502)
            index = choice.get("index", 0); integer(index, 0, 3)
            delta = choice.get("delta", {})
            require(isinstance(delta, dict), "Invalid stream delta", "model_response_rejected", 502)
            for field in ("content", "reasoning_content", "refusal"):
                if isinstance(delta.get(field), str): streams.setdefault((index, field), []).append(delta[field])
            for call in delta.get("tool_calls") or []:
                require(isinstance(call, dict), "Invalid stream tool", "model_response_rejected", 502)
                tool_index = call.get("index", 0); integer(tool_index, 0, 15)
                function = call.get("function") or {}
                require(isinstance(function, dict), "Invalid stream function", "model_response_rejected", 502)
                for field in ("name", "arguments"):
                    if isinstance(function.get(field), str): streams.setdefault((index, tool_index, field), []).append(function[field])
            legacy = delta.get("function_call") or {}
            if isinstance(legacy, dict):
                for field in ("name", "arguments"):
                    if isinstance(legacy.get(field), str): streams.setdefault((index, "legacy", field), []).append(legacy[field])
    for fragments in streams.values(): check("".join(fragments))


def abort_socket(connection):
    try: connection.shutdown(socket.SHUT_RDWR)
    except OSError: pass
    try: connection.close()
    except OSError: pass


class HeaderBoundHandler(http.server.BaseHTTPRequestHandler):
    """Absolute timers cover request-line/headers before any buffered reads."""
    def setup(self):
        self._io_timer = None
        self.request.settimeout(self.server.header_timeout)
        super().setup()
        self.set_io_timeout(self.server.header_timeout)

    def set_io_timeout(self, timeout):
        if self._io_timer:
            self._io_timer.cancel(); self._io_timer.join()
        require(timeout > 0, "Request deadline exceeded", "deadline_exceeded", 410)
        self.connection.settimeout(timeout)
        self._io_timer = threading.Timer(timeout, abort_socket, args=(self.connection,))
        self._io_timer.daemon = True; self._io_timer.start()

    def parse_request(self):
        parsed = super().parse_request()
        if parsed: self.set_io_timeout(10)
        return parsed

    def finish(self):
        if self._io_timer:
            self._io_timer.cancel(); self._io_timer.join()
        super().finish()


class OwnedUpstream:
    def __init__(self, relay, deadline):
        self.relay = relay; self.lock = threading.Lock(); self.sockets = set(); self.aborted = False
        self.deadline = deadline
        self.timer = threading.Timer(max(0, deadline - time.monotonic()), self.abort)
        self.timer.daemon = True
        with relay.active:
            require(not relay.closing, "Relay is closing", "run_inactive", 409)
            relay.upstreams.add(self)
        self.timer.start()

    def add(self, connection):
        with self.lock:
            if self.aborted:
                abort_socket(connection)
                raise Fault("deadline_exceeded", "Upstream request interrupted", 410)
            self.sockets.add(connection)
        return connection

    def abort(self):
        with self.lock:
            self.aborted = True; connections = list(self.sockets)
        for connection in connections: abort_socket(connection)

    def remaining(self):
        remaining = self.deadline - time.monotonic()
        require(not self.aborted and remaining > 0, "Upstream request interrupted", "deadline_exceeded", 410)
        return remaining

    def close(self):
        self.timer.cancel(); self.abort(); self.timer.join()
        with self.relay.active:
            self.relay.upstreams.discard(self); self.relay.active.notify_all()


class BoundedServerMixIn(socketserver.ThreadingMixIn):
    daemon_threads = True
    block_on_close = False
    header_timeout = 5.0
    max_connections = 16

    def __init__(self, address, handler):
        self.active = threading.Condition(); self.upstreams = set(); self.clients = set(); self.closing = False
        self.capacity = threading.BoundedSemaphore(self.max_connections)
        super().__init__(address, handler)

    def process_request(self, request, address):
        if not self.capacity.acquire(blocking=False): request.close(); return
        with self.active:
            if self.closing:
                self.capacity.release(); request.close(); return
            self.clients.add(request)
        try: super().process_request(request, address)
        except BaseException:
            request.close()
            with self.active:
                self.capacity.release(); self.clients.discard(request); self.active.notify_all()
            raise

    def process_request_thread(self, request, address):
        try: super().process_request_thread(request, address)
        finally:
            with self.active:
                self.capacity.release(); self.clients.discard(request); self.active.notify_all()

    def handle_error(self, *_): pass  # Never log untrusted request contents.

    def abort_active(self):
        with self.active:
            self.closing = True; upstreams, clients = list(self.upstreams), list(self.clients)
        for upstream in upstreams: upstream.abort()
        for client in clients: abort_socket(client)

    def wait_handlers(self, timeout=2):
        with self.active:
            return self.active.wait_for(lambda: not self.clients and not self.upstreams, timeout)

    def server_close(self):
        self.abort_active()
        super().server_close()
        require(self.wait_handlers(), "Connection cleanup did not complete", "connection_cleanup_failed", 503)


class RelayServer(BoundedServerMixIn, socketserver.UnixStreamServer):
    max_connections = 8


class OwnedHTTPConnection(http.client.HTTPConnection):
    """Numeric startup-resolved dial; every connect/TLS/read socket is owned."""
    def __init__(self, config, owner):
        url = urlsplit(config.gateway_url)
        super().__init__(url.hostname, url.port or (443 if url.scheme == "https" else 80), timeout=owner.remaining())
        self.config, self.owner, self.secure = config, owner, url.scheme == "https"

    def connect(self):
        require(self.config._gateway_address is not None, "Gateway address not verified", "runtime_unavailable", 503)
        family, address = self.config._gateway_address
        connection = self.owner.add(socket.socket(family, socket.SOCK_STREAM))
        connection.settimeout(self.owner.remaining()); connection.connect(address)
        if self.secure:
            # Handshake is separate so cancellation owns the TLS socket before
            # it can block. The original hostname still verifies the certificate.
            connection = self.owner.add(ssl.create_default_context().wrap_socket(
                connection, server_hostname=self.host, do_handshake_on_connect=False))
            connection.settimeout(self.owner.remaining()); connection.do_handshake()
        self.sock = connection


class RelayHandler(HeaderBoundHandler):
    protocol_version = "HTTP/1.0"
    def log_message(self, *_): pass

    def do_POST(self):
        try:
            require(self.path == "/v1/chat/completions" and not self.headers.get("Transfer-Encoding"), "Relay route denied", "relay_denied", 403)
            size = integer(int(self.headers.get("Content-Length", "0")), 1, 2_000_000)
            self.connection.settimeout(10)
            raw = self.rfile.read(size); require(len(raw) == size, "Incomplete provider request")
            body = decode(raw); require(isinstance(body, dict), "Invalid provider body")
            allowed_fields = {"model", "messages", "tools", "tool_choice", "stream", "max_tokens", "max_completion_tokens",
                "temperature", "top_p", "frequency_penalty", "presence_penalty", "parallel_tool_calls", "reasoning_effort",
                "stream_options", "stop", "seed", "user", "prompt_cache_key", "reasoning"}
            require(body.keys() <= allowed_fields, "Provider request contains unapproved options", "relay_denied", 403)
            require(body.get("model") == self.server.model_id, "Provider model substitution denied", "relay_denied", 403)
            for tool in body.get("tools", []):
                require(isinstance(tool, dict) and tool.get("type") == "function"
                        and tool.get("function", {}).get("name") in self.server.allowed_tools, "Provider tool denied", "relay_denied", 403)
            admission = self.server.store.reserve_model_request(self.server.run_id, self.server.config.daily_limit)
            for key in ("max_tokens", "max_completion_tokens"):
                if key in body: integer(body[key], 1, admission["maxOutputTokens"])
            if not any(k in body for k in ("max_tokens", "max_completion_tokens")):
                body["max_tokens"] = admission["maxOutputTokens"]
            timeout = (instant(admission["deadline"]) - datetime.now(UTC)).total_seconds()
            self.set_io_timeout(timeout)
            config = self.server.config
            owner = OwnedUpstream(self.server, time.monotonic() + timeout)
            connection = response = None
            try:
                connection = OwnedHTTPConnection(config, owner)
                headers = {"Content-Type": "application/json"}
                if config.gateway_key: headers["Authorization"] = "Bearer " + config.gateway_key
                connection.request("POST", urlsplit(config.gateway_url).path + "/chat/completions", canonical(body).encode(), headers)
                response = connection.getresponse()
                require(response.status == 200, "Model gateway failed", "model_gateway_failed", 502)
                # No redirects/proxies. Retain all bytes in the parent until the
                # reflection guard passes; read1 plus the owned socket timer
                # bounds both idle and trickling headers/chunked response bodies.
                data = bytearray()
                while True:
                    owner.remaining()
                    chunk = response.read1(min(65536, MAX_PROVIDER_BYTES + 1 - len(data)))
                    if not chunk: break
                    data.extend(chunk)
                    require(len(data) <= MAX_PROVIDER_BYTES, "Model response too large", "model_response_too_large", 502)
                content_type = response.headers.get("Content-Type", "application/json").split(";")[0]
                require(content_type in ("application/json", "text/event-stream"), "Unexpected model response content type")
                guard_model_response(data, content_type, config.gateway_key)
                owner.remaining()
                with self.server.store.lock:
                    self.server.store.fence(self.server.store.get("run", self.server.run_id))
            finally:
                owner.close()
                if response: response.close()
                if connection: connection.close()
            self.send_response(200); self.send_header("Content-Type", content_type); self.send_header("Content-Length", str(len(data)))
            self.end_headers(); self.wfile.write(data)
        except Exception as exc:
            fault = exc if isinstance(exc, Fault) else Fault("model_gateway_failed", "Approved model gateway request failed", 502)
            data = canonical(fault.wire()).encode()
            try:
                self.send_response(fault.status); self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data))); self.end_headers(); self.wfile.write(data)
            except (OSError, ValueError): pass

    def do_GET(self): self.send_error(403)
    def do_CONNECT(self): self.send_error(403)


def sandbox_command(config, relay_path):
    if sys.platform == "darwin":
        from .platforms import mac_command
        return mac_command(config, relay_path)
    require(sys.platform == "linux", "Worker sandbox unavailable", "platform_sandbox_unavailable", 503)
    source, python = config.source, config.python
    runtime = Path(__file__).resolve().parent.parent
    # Dedicated venv executable can be a symlink; mount its whole venv at the
    # same path, not merely the resolved system binary.
    venv = python.parent.parent
    command = [shutil.which("bwrap") or "bwrap", "--unshare-all", "--die-with-parent", "--new-session",
               "--cap-drop", "ALL", "--clearenv", "--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp",
               "--tmpfs", "/home", "--dir", "/home/forma", "--dir", "/home/forma/hermes", "--dir", "/run"]
    for path in ("/usr", "/lib", "/lib64"):
        if Path(path).exists(): command += ["--ro-bind", path, path]
    for path in (source, venv, runtime): command += ["--ro-bind", str(path), str(path)]
    command += ["--ro-bind", str(relay_path), "/run/forma-relay.sock", "--chdir", "/home/forma",
                "--setenv", "HOME", "/home/forma", "--setenv", "HERMES_HOME", "/home/forma/hermes",
                "--setenv", "HERMES_SAFE_MODE", "1", "--setenv", "FORMA_HERMES_SOURCE", str(source),
                "--setenv", "PYTHONPATH", str(runtime), "--setenv", "PYTHONDONTWRITEBYTECODE", "1",
                "--setenv", "PYTHONUNBUFFERED", "1", "--setenv", "PATH", "/usr/bin",
                "--setenv", "LANG", "C.UTF-8", str(python), "-B", "-m", "forma_runtime.worker"]
    return command


class Supervisor:
    def __init__(self, store, config):
        self.store, self.config = store, config
        self.stopping = threading.Event(); self.thread = None

    def start(self):
        self.store.recover()
        self.thread = threading.Thread(target=self.loop, name="forma-supervisor", daemon=True); self.thread.start()

    def stop(self):
        self.stopping.set()
        if self.thread:
            self.thread.join(timeout=8)
            require(not self.thread.is_alive(), "Worker cleanup did not complete", "worker_cleanup_failed", 503)

    def loop(self):
        while not self.stopping.is_set():
            try:
                if not self.config.ready():
                    self.stopping.wait(.25); continue
                self.store.tick()
                run = self.store.start_next()
                if run: self.execute(run)
                else: self.stopping.wait(0.25)
            except Exception:
                # A failed scheduler pass must not silently admit another copy.
                self.stopping.wait(1)

    def check_run(self, rid, deadline):
        require(not self.stopping.is_set(), "Supervisor stopping", "supervisor_stopped", 409)
        with self.store.lock:
            current = self.store.get("run", rid)
            require(current["state"] != "cancelling", "Run cancelled", "cancelled", 409)
            try:
                require(time.monotonic() < deadline, "Run deadline exceeded", "deadline_exceeded", 410)
                self.store.fence(current)
            except Fault as exc:
                if current["state"] == "waiting_for_device" and exc.code == "deadline_exceeded":
                    raise Fault("device_unavailable", "Native tool deadline exceeded", 410) from exc
                raise
            return current

    def execute(self, run):
        process = relay = temporary = relay_thread = memory_guard = None
        shared_group = self.config.managed and sys.platform == "darwin"
        failure, result = None, None
        try:
            # Everything after start_next(), including grant and setup failures,
            # belongs to this terminal boundary. Wall-clock fences are retained;
            # monotonic time prevents clock changes from extending active I/O.
            deadline = time.monotonic() + (instant(run["_deadline"]) - datetime.now(UTC)).total_seconds()
            def check():
                self.check_run(run["id"], deadline)
                if memory_guard and process.poll() is None: memory_guard.check()
            check()
            allowed = set(TOOLS) - set(GOOGLE_TOOLS)
            with self.store.lock:
                grants = self.store.check_grants(run["workspaceId"], run["modelId"], run["_input"]["grantRefs"])
                for grant in grants: allowed.update(grant["operations"])
            if "scheduleId" in run: allowed.discard("forma_schedule_create")
            temporary = tempfile.TemporaryDirectory(prefix="forma-worker-")
            relay_path = Path(temporary.name) / "gateway.sock"
            relay = RelayServer(str(relay_path), RelayHandler)
            relay.store, relay.config, relay.run_id = self.store, self.config, run["id"]
            relay.model_id, relay.allowed_tools = run["modelId"], allowed
            relay_thread = threading.Thread(target=relay.serve_forever, kwargs={"poll_interval": .05}, daemon=True)
            relay_thread.start()
            os.chmod(relay_path, 0o600); check()
            process = subprocess.Popen(sandbox_command(self.config, relay_path), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                stderr=subprocess.DEVNULL, env={"PATH": "/usr/bin:/bin"}, start_new_session=not shared_group, bufsize=0,
                cwd=relay_path.parent / "home" if sys.platform == "darwin" else None)
            if sys.platform == "darwin":
                from .platforms import MacMemoryGuard
                memory_guard = MacMemoryGuard(process.pid)
            os.set_blocking(process.stdin.fileno(), False); os.set_blocking(process.stdout.fileno(), False)
            job = {"type": "start", "runId": run["id"], "modelId": run["modelId"], "message": run["_input"]["message"],
                   "history": run["_history"], "budgets": run["budgets"], "allowedTools": sorted(allowed)}
            if "scheduleId" in run:
                with self.store.lock:
                    schedule = self.store.get("schedule", run["scheduleId"])
                    target = self.store.get("interface", schedule["interfaceId"])
                job["scheduleTarget"] = {"interfaceId": target["id"], "expectedRevision": run["_targetRevision"], "spec": target["spec"]}
            self.send(process, job, check, deadline)
            tool_attempts = 0; delta_bytes = 0; pending = bytearray()
            while True:
                check()
                newline = pending.find(b"\n")
                if newline < 0:
                    require(len(pending) <= MAX_FRAME, "Worker frame too large", "worker_frame_exceeded", 429)
                    readable, _, _ = select.select([process.stdout], [], [], .05)
                    if not readable: continue
                    try: chunk = os.read(process.stdout.fileno(), min(65536, MAX_FRAME + 1 - len(pending)))
                    except BlockingIOError: continue
                    if not chunk: failure = "worker_exited"; break
                    pending.extend(chunk); continue
                require(newline + 1 <= MAX_FRAME, "Worker frame too large", "worker_frame_exceeded", 429)
                frame = worker_frame(decode(pending[:newline])); del pending[:newline + 1]
                if frame.get("type") == "delta":
                    text(frame.get("text"), 16000); delta_bytes += len(frame["text"].encode())
                    require(delta_bytes <= 262144, "Assistant output budget exceeded", "budget_exceeded", 429)
                    self.store.delta(run["id"], frame["text"])
                elif frame.get("type") == "tool":
                    tool_attempts += 1
                    require(tool_attempts <= run["budgets"]["maxToolCalls"], "Tool attempt budget exceeded", "budget_exceeded", 429)
                    try:
                        answer = self.store.tool(run["id"], frame.get("name"), frame.get("args"))
                        if "_waitRequest" in answer:
                            request_id = answer["_waitRequest"]
                            while True:
                                check(); answer = self.store.tool_result(run["id"], request_id)
                                if answer is not None: break
                                self.stopping.wait(.05)
                        self.send(process, {"type": "tool-result", "result": answer}, check, deadline)
                    except Fault as exc:
                        self.send(process, {"type": "tool-result", "result": exc.wire()}, check, deadline)
                elif frame.get("type") == "result": result = frame.get("result"); break
                else: failure = frame.get("code", "worker_protocol_failed"); break
        except Exception as exc:
            failure = exc if isinstance(exc, Fault) else "worker_launch_failed"
        finally:
            # Abort egress before reaping, even if no worker was launched. A
            # daemon handler is not cleanup: close owns and drains its sockets.
            if relay: relay.abort_active()
            try:
                if process:
                    if process.poll() is None:
                        try:
                            if shared_group: process.terminate()
                            else: os.killpg(process.pid, signal.SIGTERM)
                        except ProcessLookupError: pass
                        try: process.wait(timeout=2)
                        except subprocess.TimeoutExpired:
                            try:
                                if shared_group: process.kill()
                                else: os.killpg(process.pid, signal.SIGKILL)
                            except ProcessLookupError: pass
                            process.wait(timeout=2)
            except Exception:
                failure = "worker_cleanup_failed"; self.stopping.set()
            finally:
                if process:
                    if process.stdin: process.stdin.close()
                    if process.stdout: process.stdout.close()
                try:
                    if relay:
                        if relay_thread and relay_thread.is_alive(): relay.shutdown()
                        relay.server_close()
                        if relay_thread: relay_thread.join(timeout=1)
                    if temporary: temporary.cleanup()
                except Exception:
                    failure = "connection_cleanup_failed"; self.stopping.set()
                finally:
                    self.store.finish(run["id"], result=result, failure=failure)

    @staticmethod
    def send(process, body, check, deadline):
        raw = memoryview(canonical(body).encode() + b"\n")
        require(len(raw) <= MAX_FRAME, "Supervisor frame too large", "budget_exceeded", 429)
        while raw:
            check()
            remaining = deadline - time.monotonic()
            require(remaining > 0, "Run deadline exceeded", "deadline_exceeded", 410)
            _, writable, _ = select.select([], [process.stdin], [], min(.05, remaining))
            if not writable: continue
            try: sent = os.write(process.stdin.fileno(), raw[:65536])
            except BlockingIOError: continue
            require(sent > 0, "Worker pipe closed", "worker_exited", 409)
            raw = raw[sent:]
        check()
