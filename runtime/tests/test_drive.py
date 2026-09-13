"""Browser-drive tests: LOCAL fixture CDP targets only, never live sites.

Policy units run anywhere. The integration case drives drive.py's real
patchright connect-over-CDP path against the ALREADY-BUNDLED Chromium
headless shell (no download in tests); without patchright or the shell it
skips — a skip is never a PASS.
"""
import asyncio
import functools
import http.server
import json
import os
import socketserver
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from forma_runtime.contracts import BROWSER_TOOLS, Fault, browser_result  # noqa: E402
from forma_runtime.drive import (  # noqa: E402
    CLICK_HOLD_MS,
    KEYSTROKE_DELAY_MS,
    MAX_SNAPSHOT_TEXT,
    PACING_PAUSE_SECONDS,
    BrowserDrive,
    cdp_endpoint,
    click_hold_ms,
    drive_url,
    keystroke_delay_ms,
    valid_selector,
)

REPO = Path(__file__).resolve().parents[2]


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


def patchright_available():
    try:
        import patchright  # noqa: F401
    except ImportError:
        return False
    return True


class DrivePolicyCase(unittest.TestCase):
    def test_cdp_endpoint_is_loopback_only(self):
        self.assertEqual(cdp_endpoint("http://127.0.0.1:9222"), "http://127.0.0.1:9222")
        self.assertEqual(cdp_endpoint("http://127.0.0.1:9222/"), "http://127.0.0.1:9222")
        for value in [
            "http://localhost:9222",
            "http://10.0.0.1:9222",
            "https://127.0.0.1:9222",
            "http://127.0.0.1",
            "http://user:pass@127.0.0.1:9222",
            "http://127.0.0.1:9222/path",
            "ws://127.0.0.1:9222",
        ]:
            with self.assertRaises(Fault):
                cdp_endpoint(value)

    def test_drive_destinations(self):
        for url in ["https://example.com/", "https://example.com/path?x=1", "http://example.org/"]:
            self.assertEqual(drive_url(url), url)
        for url in [
            "https://higgsfield.ai/",
            "https://cdn.HIGGSFIELD.io/",
            "http://localhost/",
            "http://127.0.0.1:8000/",
            "http://192.168.1.4/",
            "https://printer.local/",
            "https://portal.internal/",
            "https://example.com:8080/",
            "https://user:secret@example.com/",
            "javascript:alert(1)",
            "file:///tmp/secret",
        ]:
            with self.assertRaises(Fault):
                drive_url(url)
        with self.assertRaises(Fault) as frozen:
            drive_url("https://higgsfield.ai/")
        self.assertEqual(frozen.exception.code, "destination_frozen")
        # Local fixture mode is loopback http only — never a bypass for the web.
        self.assertEqual(drive_url("http://127.0.0.1:8000/x", True), "http://127.0.0.1:8000/x")
        for url in ["https://example.com/", "http://localhost:8000/"]:
            with self.assertRaises(Fault):
                drive_url(url, True)

    def test_selectors_are_bounded(self):
        self.assertEqual(valid_selector("#main input[name=q]"), "#main input[name=q]")
        for value in ["", "x" * 501, "a\nb"]:
            with self.assertRaises(Fault):
                valid_selector(value)

    def test_human_pacing_is_randomized_within_bounds(self):
        # The anti-ban cadence must stay in its declared human-like ranges.
        self.assertGreaterEqual(PACING_PAUSE_SECONDS[0], 0.2)
        self.assertLessEqual(PACING_PAUSE_SECONDS[1], 2.0)
        keys = {keystroke_delay_ms() for _ in range(200)}
        self.assertTrue(all(KEYSTROKE_DELAY_MS[0] <= k <= KEYSTROKE_DELAY_MS[1] for k in keys))
        self.assertGreater(len(keys), 5, "keystroke delay must actually vary")
        holds = {click_hold_ms() for _ in range(200)}
        self.assertTrue(all(CLICK_HOLD_MS[0] <= h <= CLICK_HOLD_MS[1] for h in holds))
        self.assertGreater(len(holds), 5, "click hold must actually vary")

    def test_browser_session_results_are_loopback_and_leak_free(self):
        request = {"toolName": BROWSER_TOOLS[0], "args": {}}
        browser_result({"kind": "browserSession", "available": True,
                        "cdpEndpoint": "http://127.0.0.1:9222"}, request)
        browser_result({"kind": "browserSession", "available": False,
                        "detail": "The user has not enabled assistant browser driving."}, request)
        for invalid in (
            {"kind": "browserSession", "available": True},  # available needs endpoint
            {"kind": "browserSession", "available": False,
             "cdpEndpoint": "http://127.0.0.1:9222"},  # unavailable must not leak one
            {"kind": "browserSession", "available": True,
             "cdpEndpoint": "http://10.0.0.1:9222"},  # never a remote endpoint
            {"kind": "browserSession", "available": True,
             "cdpEndpoint": "ws://127.0.0.1:9222"},
            {"kind": "connectedServices", "available": True,
             "cdpEndpoint": "http://127.0.0.1:9222"},  # wrong kind
            {"kind": "browserSession", "available": "yes"},
        ):
            with self.subTest(invalid=invalid):
                with self.assertRaises(Fault):
                    browser_result(invalid, request)
        with self.assertRaises(Fault):
            browser_result({"kind": "browserSession", "available": False},
                           {"toolName": "forma_list_connected_services", "args": {}})

    def test_unstarted_drive_never_touches_a_browser(self):
        drive = BrowserDrive("http://127.0.0.1:9222")
        with self.assertRaises(Fault):
            asyncio.run(drive.snapshot())


FIXTURE_PAGE = """<!doctype html><html><head><title>Drive Fixture</title></head>
<body><h1>Heading</h1><input id="query" type="text">
<a id="next" href="/second.html">second</a></body></html>"""
FIXTURE_SECOND = """<!doctype html><html><head><title>Second Drive</title></head>
<body><p>Second page.</p></body></html>"""


@unittest.skipUnless(patchright_available() and chromium_executable(),
                     "patchright install and bundled chromium shell required")
class DriveCdpCase(unittest.TestCase):
    """Real connect_over_cdp against the bundled headless shell, serving only
    local fixture pages. No download, no external destination."""

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
        cls.shell = subprocess.Popen(
            [str(chromium_executable()), "--remote-debugging-port=0",
             f"--user-data-dir={cls.profile.name}", "--no-first-run",
             "--no-default-browser-check", "about:blank"],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        marker = Path(cls.profile.name) / "DevToolsActivePort"
        deadline = time.monotonic() + 60
        cdp_port = None
        while time.monotonic() < deadline:
            if marker.is_file():
                first = marker.read_text().splitlines()
                if first and first[0].strip().isdigit() and int(first[0]) > 0:
                    cdp_port = int(first[0])
                    break
            time.sleep(0.2)
        if cdp_port is None:
            raise AssertionError("headless shell never published its CDP port")
        cls.endpoint = f"http://127.0.0.1:{cdp_port}"

    @classmethod
    def tearDownClass(cls):
        cls.server.shutdown()
        cls.shell.terminate()
        try:
            cls.shell.wait(timeout=10)
        except subprocess.TimeoutExpired:
            cls.shell.kill()
        cls.directory.cleanup()
        cls.profile.cleanup()

    def drive(self, operation):
        async def run():
            async with BrowserDrive(self.endpoint, allow_local=True) as drive:
                return await operation(drive)
        return asyncio.run(run())

    def test_navigate_snapshot_type_and_click_are_bounded_and_paced(self):
        page_url = f"http://127.0.0.1:{self.port}/page.html"

        async def scenario(drive):
            opened = await drive.navigate(page_url)
            snap = await drive.snapshot()
            typed = await drive.type("#query", "hello browser")
            clicked = await drive.click("#next")
            follow = await drive.snapshot()
            return opened, snap, typed, clicked, follow

        opened, snap, typed, clicked, follow = self.drive(scenario)
        self.assertEqual(opened["status"], 200)
        self.assertEqual(opened["title"], "Drive Fixture")
        self.assertIn("Heading", snap["text"])
        self.assertLessEqual(len(snap["text"]), MAX_SNAPSHOT_TEXT)
        self.assertFalse(snap["truncated"])
        self.assertEqual(typed["typed"], len("hello browser"))
        self.assertEqual(clicked["clicked"], "#next")
        self.assertIn("Second page.", follow["text"])
        self.assertEqual(follow["title"], "Second Drive")

    def test_denied_destinations_never_reach_the_browser(self):
        async def scenario(drive):
            with self.assertRaises(Fault):
                await drive.navigate("https://higgsfield.ai/")
            with self.assertRaises(Fault):
                await drive.navigate("https://example.com/")
            return await drive.snapshot()

        snap = self.drive(scenario)
        self.assertNotIn("higgsfield", snap["url"])

    def test_detach_leaves_the_browser_running(self):
        self.drive(lambda drive: drive.snapshot())
        self.assertIsNone(self.shell.poll(), "close() must detach, not kill")


if __name__ == "__main__":
    unittest.main()
