"""Scrapling engine tests: LOCAL fixture pages only, never live targets.

Policy units run anywhere. Engine subprocess tests need the bundled Scrapling
install and Chromium shell; without them they skip — a skip is never a PASS.
"""
import asyncio
import functools
import select
import http.server
import json
import os
import socketserver
import subprocess
import sys
import tempfile
import threading
import unittest
import uuid
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from forma_runtime.contracts import Fault  # noqa: E402
from forma_runtime.engine import (  # noqa: E402
    MAX_BODY,
    Engine,
    engine_url,
    extract_title,
    relay_endpoint,
)

REPO = Path(__file__).resolve().parents[2]
ENGINE = REPO / "runtime" / "forma_runtime" / "engine.py"


def chromium_executable():
    override = os.environ.get("FORMA_ENGINE_TEST_EXECUTABLE")
    if override:
        path = Path(override)
        return path if path.is_file() else None
    bundled = REPO / "src-tauri" / "resources" / "managed-browsers"
    manifest = bundled / "manifest.json"
    if not manifest.is_file():
        return None
    executable = json.loads(manifest.read_text())["chromium"]["executable"]
    path = bundled / executable
    return path if path.is_file() else None


def scrapling_available():
    try:
        import scrapling  # noqa: F401
    except ImportError:
        return False
    return True


FIXTURE_PAGE = """<!doctype html><html><head><title>Fixture &amp; Page</title></head>
<body><h1>Heading</h1><a href="/second.html">second</a></body></html>"""
FIXTURE_SECOND = """<!doctype html><html><head><title>Second Fixture</title></head>
<body><p>Second page.</p></body></html>"""


class EnginePolicyCase(unittest.TestCase):
    def assert_denied(self, url, fixture=False, code=None):
        with self.assertRaises(Fault) as caught:
            engine_url(url, fixture)
        if code is not None:
            self.assertEqual(caught.exception.code, code)

    def test_production_destinations(self):
        for url in [
            "https://example.com/",
            "https://example.com/path?x=1",
            "http://example.org/",
        ]:
            self.assertEqual(engine_url(url, False), url)

    def test_frozen_and_local_destinations_fail_closed(self):
        for url in [
            "https://higgsfield.ai/",
            "https://cdn.HIGGSFIELD.io/",
            "http://localhost/",
            "https://printer.local/",
            "https://portal.internal/",
            "https://example.com:8080/",
            "https://user:secret@example.com/",
            "https://example.com\\@evil.test",
            "javascript:alert(1)",
            "file:///tmp/secret",
            "ftp://example.com/",
        ]:
            self.assert_denied(url)
        self.assert_denied("https://higgsfield.ai/", code="destination_frozen")

    def test_fixture_mode_is_loopback_http_only(self):
        self.assertEqual(engine_url("http://127.0.0.1:8000/page.html", True),
                         "http://127.0.0.1:8000/page.html")
        for url in [
            "http://localhost/page.html",
            "https://127.0.0.1:8000/page.html",
            "https://example.com/",
        ]:
            self.assert_denied(url, fixture=True)

    def test_relay_endpoint(self):
        self.assertEqual(relay_endpoint("http://127.0.0.1:49152"), "http://127.0.0.1:49152")
        for value in [
            "http://127.0.0.1",
            "http://10.0.0.1:49152",
            "http://127.0.0.1:49152/path",
            "https://127.0.0.1:49152",
            "http://user:pass@127.0.0.1:49152",
        ]:
            with self.assertRaises(Fault):
                relay_endpoint(value)

    def test_title_extraction(self):
        self.assertEqual(extract_title(FIXTURE_PAGE), "Fixture & Page")
        self.assertEqual(extract_title("<html><TITLE>mixed &lt;case&gt;</TITLE></html>"), "mixed <case>")
        self.assertEqual(extract_title("<p>no title</p>"), "")
        self.assertEqual(extract_title("<title>" + "x" * 900 + "</title>"), "x" * 500)


@unittest.skipUnless(scrapling_available() and chromium_executable(),
                     "bundled scrapling install and chromium shell required")
class EngineSubprocessCase(unittest.TestCase):
    """Drives the real engine child over its stdio protocol against a local
    fixture server. No relay: bootstrap uses the test-only fixtureRoot flag."""

    @classmethod
    def setUpClass(cls):
        cls.directory = tempfile.TemporaryDirectory()
        root = Path(cls.directory.name)
        (root / "page.html").write_text(FIXTURE_PAGE, encoding="utf-8")
        (root / "second.html").write_text(FIXTURE_SECOND, encoding="utf-8")
        handler = functools.partial(http.server.SimpleHTTPRequestHandler, directory=str(root))
        server = socketserver.ThreadingTCPServer(("127.0.0.1", 0), handler)
        server.daemon_threads = True
        cls.port = server.server_address[1]
        cls.server = server
        threading.Thread(target=server.serve_forever, daemon=True).start()

        cls.profile = tempfile.TemporaryDirectory()
        cls.child = subprocess.Popen(
            [sys.executable, "-I", "-B", str(ENGINE)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            cwd=cls.profile.name,
        )
        cls.sequence = 0

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        try:
            if cls.child.poll() is None:
                cls.child.stdin.close()
                cls.child.wait(timeout=10)
        finally:
            cls.directory.cleanup()
            cls.profile.cleanup()

    @classmethod
    def call(cls, timeout=90, **frame):
        cls.sequence += 1
        frame["id"] = str(uuid.uuid4())
        request = (json.dumps(frame) + "\n").encode()
        cls.child.stdin.write(request)
        cls.child.stdin.flush()
        import time as _time
        end = _time.monotonic() + timeout
        payload = bytearray()
        while b"\n" not in payload:
            if _time.monotonic() > end:
                raise AssertionError("engine response timed out")
            readable, _, _ = select.select([cls.child.stdout], [], [], 0.2)
            if readable:
                chunk = os.read(cls.child.stdout.fileno(), 65536)
                if not chunk:
                    break
                payload.extend(chunk)
        line, separator, rest = bytes(payload).partition(b"\n")
        assert separator
        return json.loads(line)

    def test_01_bootstrap_describes_read_only_capabilities(self):
        reply = self.call(op="bootstrap", browsersExecutable=str(chromium_executable()),
                          userDataDir=self.profile.name, fixtureRoot=self.directory.name)
        self.assertTrue(reply["ok"], reply)
        result = reply["result"]
        self.assertEqual(result["protocol"], "forma-engine-v1")
        self.assertTrue(result["fixtureMode"])
        self.assertFalse(result["capabilities"]["interactive"])

    def test_02_navigate_fetches_local_fixture(self):
        reply = self.call(op="navigate", url=f"http://127.0.0.1:{self.port}/page.html")
        self.assertTrue(reply["ok"], reply)
        page = reply["result"]
        self.assertEqual(page["status"], 200)
        self.assertEqual(page["title"], "Fixture & Page")
        self.assertIn("Heading", page["html"])
        self.assertFalse(page["interactive"])
        self.assertFalse(page["truncated"])

    def test_03_navigate_again_reuses_the_session(self):
        reply = self.call(op="navigate", url=f"http://127.0.0.1:{self.port}/second.html")
        self.assertTrue(reply["ok"], reply)
        self.assertEqual(reply["result"]["title"], "Second Fixture")

    def test_04_fetch_uses_the_plain_fetcher(self):
        reply = self.call(op="fetch", url=f"http://127.0.0.1:{self.port}/second.html")
        self.assertTrue(reply["ok"], reply)
        body = reply["result"]
        self.assertEqual(body["status"], 200)
        self.assertIn("Second page.", body["body"])
        self.assertIn("text/html", body["contentType"])

    def test_05_denied_destinations_fail_without_contact(self):
        for url in ["https://higgsfield.ai/", "https://example.com/", "http://10.0.0.1/"]:
            reply = self.call(op="navigate", url=url)
            self.assertFalse(reply["ok"], url)
            self.assertIn(reply["error"]["code"], ("destination_denied", "destination_frozen", "invalid_request"))

    def test_06_oversized_snapshot_is_truncated(self):
        # A page beyond the engine's body bound must come back capped and flagged.
        Path(self.directory.name, "big.html").write_text(
            "<html><head><title>Big</title></head><body>" + "y" * (MAX_BODY + 4096) + "</body></html>",
            encoding="utf-8")
        reply = self.call(op="navigate", url=f"http://127.0.0.1:{self.port}/big.html")
        self.assertTrue(reply["ok"], reply)
        result = reply["result"]
        self.assertTrue(result["truncated"])
        self.assertEqual(len(result["html"]), MAX_BODY)

    def test_07_close_stops_cleanly(self):
        reply = self.call(op="close")
        self.assertTrue(reply["ok"], reply)
        self.assertTrue(reply["result"]["stopped"])
        self.assertEqual(self.child.wait(timeout=15), 0)


class EngineUnitCase(unittest.TestCase):
    def test_dispatch_requires_relay_or_fixture(self):
        engine = Engine()
        with tempfile.TemporaryDirectory() as directory:
            marker = Path(directory) / "shell"
            marker.write_bytes(b"#!/bin/sh\n")
            marker.chmod(0o755)
            with self.assertRaises(Fault) as caught:
                asyncio.run(engine.dispatch({"id": str(uuid.uuid4()), "op": "bootstrap",
                                             "browsersExecutable": str(marker),
                                             "userDataDir": directory}))
            self.assertEqual(caught.exception.code, "relay_required")

    def test_navigate_result_is_bounded(self):
        engine = Engine()
        engine.config = {"browsers": Path("/nonexistent"), "profile": Path(tempfile.gettempdir()),
                         "proxy": None, "fixture": False}

        class FakePage:
            status, url, encoding = 200, "https://example.com/", ""
            html_content = "z" * (MAX_BODY + 10)

        class FakeSession:
            async def fetch(self, _url, **_kwargs):
                return FakePage()

        async def fake_session(_engine):
            return FakeSession()

        with patch.object(Engine, "ensure_session", fake_session):
            result = asyncio.run(engine.dispatch({"id": str(uuid.uuid4()), "op": "navigate",
                                                  "url": "https://example.com/"}))
        self.assertLessEqual(len(result["html"]), MAX_BODY)
        self.assertTrue(result["truncated"])


if __name__ == "__main__":
    unittest.main()
