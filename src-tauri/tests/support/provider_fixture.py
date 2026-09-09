"""Synthetic provider for explicit external-fixture QA; run on the designated test host only.

Binds loopback. No credentials, real models, account access, or external requests.
The ignored Rust test reaches it through an explicitly created SSH tunnel.
"""
import argparse
import json
import os
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

EVENTS = []
LOCK = threading.Lock()
LOG_PATH = None
ALLOWED_INPUTS = {
    "forma-fixture:success",
    "forma-fixture:error",
    "forma-fixture:malformed",
    "forma-fixture:slow-cancel",
    "forma-fixture:slow-delete",
    "forma-fixture:slow-config",
}


def record(event):
    with LOCK:
        if len(EVENTS) >= 32:
            raise RuntimeError("Bounded fixture request limit reached")
        EVENTS.append(event)


def finish(event):
    with LOCK:
        with open(LOG_PATH, "a", encoding="utf-8") as log:
            log.write(json.dumps(event, separators=(",", ":")) + "\n")


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_args):
        pass  # Never use default access logs, headers, or raw remote errors.

    def send_json(self, status, body, event=None):
        payload = json.dumps(body, separators=(",", ":")).encode()
        if event is not None:
            event["status"] = status
            event["response"] = body
        try:
            self.send_response(status)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)
        except (BrokenPipeError, ConnectionResetError):
            if event is not None:
                event["clientDisconnected"] = True
        finally:
            if event is not None:
                finish(event)

    def do_GET(self):
        if self.path == "/v1/receipt":
            with LOCK:
                body = {"fixture": "forma-synthetic-v1", "requests": list(EVENTS)}
            self.send_json(200, body)
        elif self.path == "/v1/models":
            event = {"method": "GET", "path": "/v1/models", "request": None}
            record(event)
            self.send_json(200, {"object": "list", "data": [{"id": "forma-fixture", "object": "model"}]}, event)
        else:
            self.send_json(404, {"error": "Fixture route not found"})

    def do_POST(self):
        if self.path != "/v1/chat/completions":
            self.send_json(404, {"error": "Fixture route not found"})
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
            if not 0 < length <= 131072:
                raise ValueError()
            body = json.loads(self.rfile.read(length))
            messages = body["messages"]
            last = next(message["content"] for message in reversed(messages) if message["role"] == "user")
            if last not in ALLOWED_INPUTS or body["model"] != "forma-fixture":
                raise ValueError()
            if body.get("stream") is not False or body.get("max_tokens") != 2048:
                raise ValueError()
            roles = [message["role"] for message in messages]
            if not roles or roles[0] != "system" or any(role not in ("system", "user", "assistant") for role in roles):
                raise ValueError()
        except (ValueError, KeyError, TypeError, StopIteration):
            self.send_json(400, {"error": "Only bounded synthetic fixture inputs are accepted"})
            return
        # Whitelist request fields. Never log Authorization or arbitrary message text.
        event = {"method": "POST", "path": self.path, "lastUser": last,
                 "request": {"model": "forma-fixture", "stream": False, "max_tokens": 2048, "messageRoles": roles, "lastUser": last}}
        record(event)
        if last.startswith("forma-fixture:slow-"):
            time.sleep(2)
        if last == "forma-fixture:error":
            self.send_json(503, {"error": {"message": "fixture-only-diagnostic-do-not-expose"}}, event)
        elif last == "forma-fixture:malformed":
            self.send_json(200, {"unexpected": "fixture-only-diagnostic-do-not-expose"}, event)
        else:
            self.send_json(200, {"id": "forma-fixture-reply", "object": "chat.completion", "choices": [
                {"index": 0, "finish_reason": "stop", "message": {"role": "assistant", "content": "Synthetic Forma fixture reply."}}
            ]}, event)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--receipt-log", required=True)
    args = parser.parse_args()
    if os.environ.get("FORMA_SYNTHETIC_FIXTURE") != "1":
        raise SystemExit("Explicit FORMA_SYNTHETIC_FIXTURE=1 required; run only on the designated test host")
    os.umask(0o077)
    LOG_PATH = args.receipt_log
    with open(LOG_PATH, "x", encoding="utf-8"):
        pass
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    server.daemon_threads = True
    print(json.dumps({"fixture": "forma-synthetic-v1", "bind": "127.0.0.1", "port": args.port}), flush=True)
    server.serve_forever()
