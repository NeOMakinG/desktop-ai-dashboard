"""Dedicated Forma HTTP control plane. Importing this module starts nothing."""
from __future__ import annotations

import hashlib
import hmac
import http.server
import os
import signal
import threading
from urllib.parse import parse_qs, urlsplit

from .contracts import *
from .store import Store
from .supervisor import BoundedServerMixIn, Config, HeaderBoundHandler, Supervisor


class Application:
    def __init__(self, config):
        self.config = config
        self.store = Store(config.database, config.library_id, config.device_id, config.models, config.ready)
        config.library_id, config.device_id = self.store.library_id, self.store.device_id
        self.supervisor = Supervisor(self.store, config)

    def capabilities(self):
        return {"contractVersion": VERSION, "deviceId": self.config.device_id, "libraryId": self.config.library_id,
                "modelOrigin": self.config.model_origin,
                "runtime": {"kind": "hermes", "ready": self.config.ready(), "revision": REVISION,
                            **({"reason": self.config._reason} if not self.config.ready() else {})},
                "features": {"eventPolling": True, "interfaces": True, "schedules": True, "nativeToolBridge": True,
                             "generatedCodeExecution": False, "liveGoogle": False},
                "tools": [{"name": name, "available": name not in GOOGLE_TOOLS and self.config.ready(),
                           "execution": "nativeDevice" if name in GOOGLE_TOOLS or name in COMPOSIO_TOOLS else "runtime",
                           **({"reason": "explicit_synthetic_grant_required"} if name in GOOGLE_TOOLS else {})} for name in TOOLS],
                "limits": dict(LIMITS, dailyModelRequests=self.config.daily_limit, maxQueuedRuns=8)}

    def dispatch(self, method, path, query, headers, raw):
        token = headers.get("Authorization", "")
        require(token.startswith("Bearer ") and hmac.compare_digest(
            hashlib.sha256(token[7:].encode()).hexdigest(), self.config.token_sha256),
            "Runtime authentication required", "unauthenticated", 401)
        return self.dispatch_owned(method, path, query, headers, raw)

    def dispatch_owned(self, method, path, query, headers, raw):
        """Only the app-owned pipe and authenticated fixture adapter call this."""
        if method == "GET":
            if path == "/v1/capabilities": return 200, self.capabilities()
            if path == "/v1/models": return 200, {"items": self.config.models}
            return 200, self.store.get_route(path, query)
        require(method == "POST", "Method not allowed", "method_not_allowed", 405)
        require(headers.get("Content-Type", "").split(";")[0].lower() == "application/json", "Expected JSON content type")
        body = decode(raw)
        return self.store.post(path, headers.get("Idempotency-Key"), body)

    def close(self):
        self.supervisor.stop(); self.store.close()


class Server(BoundedServerMixIn, http.server.HTTPServer):
    request_queue_size = 16
    def __init__(self, address, app):
        self.app = app
        super().__init__(address, Handler)


class Handler(HeaderBoundHandler):
    protocol_version = "HTTP/1.0"
    server_version = "FormaRuntime/1"
    def log_message(self, *_): pass  # No paths, tokens, prompts, account data.

    def handle_request(self):
        try:
            self.connection.settimeout(10)
            require(not self.headers.get("Transfer-Encoding"), "Chunked requests unsupported")
            size = integer(int(self.headers.get("Content-Length", "0")), 0, 524288)
            # Authenticate before reading untrusted body bytes.
            authorization = self.headers.get("Authorization", "")
            require(authorization.startswith("Bearer ") and hmac.compare_digest(
                hashlib.sha256(authorization[7:].encode()).hexdigest(), self.server.app.config.token_sha256),
                "Runtime authentication required", "unauthenticated", 401)
            parts = urlsplit(self.path)
            require(not parts.scheme and not parts.netloc and not parts.fragment, "Invalid request target")
            queries = parse_qs(parts.query, keep_blank_values=True)
            require(all(len(values) == 1 for values in queries.values()), "Duplicate query parameter")
            raw = self.rfile.read(size); require(len(raw) == size, "Incomplete request")
            status, value = self.server.app.dispatch(self.command, parts.path,
                {key: values[0] for key, values in queries.items()}, self.headers, raw)
        except Fault as exc: status, value = exc.status, exc.wire()
        except (ValueError, TypeError, KeyError, RecursionError, UnicodeError):
            status, value = 422, Fault("invalid_request", "Invalid request shape").wire()
        except Exception:
            status, value = 500, Fault("internal_error", "Runtime request failed", 500).wire()
        payload = canonical(value).encode()
        if len(payload) > 393216:
            status, payload = 500, canonical(Fault("response_limit", "Response exceeds protocol bound", 500).wire()).encode()
        try:
            self.send_response(status); self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload))); self.send_header("Cache-Control", "no-store")
            self.send_header("X-Content-Type-Options", "nosniff"); self.end_headers(); self.wfile.write(payload)
        except OSError: pass

    do_GET = handle_request
    do_POST = handle_request
    do_OPTIONS = handle_request
    do_PUT = handle_request
    do_DELETE = handle_request


def main():
    require(os.uname().sysname == "Linux", "Backend runs only on Linux/miniforum")
    os.umask(0o077)
    config = Config.from_env()
    config.database.parent.mkdir(parents=True, exist_ok=True)
    config.verify()
    app = Application(config); server = Server((config.bind, config.port), app)
    def stop(*_):
        threading.Thread(target=server.shutdown, daemon=True).start()
    signal.signal(signal.SIGTERM, stop); signal.signal(signal.SIGINT, stop)
    try:
        app.supervisor.start(); server.serve_forever(poll_interval=0.2)
    finally:
        server.server_close(); app.close()


if __name__ == "__main__":
    main()
