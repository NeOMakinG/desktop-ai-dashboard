"""App-owned Scrapling browser engine child; stdio control plane only.

Launch with bundled Python -I -B /owned/runtime/forma_runtime/engine.py.
Never listens on TCP and never receives credentials: all egress flows through
the native-hosted loopback relay whose loopback endpoint bootstrap supplies.
Pages render headless in the bundled Chromium shell; the native host owns the
window, the relay policy, and every secret.
"""
from __future__ import annotations

import asyncio
import importlib.metadata
import os
import re
import signal
import sys
import threading
from pathlib import Path
from urllib.parse import urlsplit

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from forma_runtime.contracts import *

PROTOCOL = "forma-engine-v1"
MAX_REQUEST = 600_000
MAX_RESPONSE = 12 * 1024 * 1024
MAX_BODY = 8 * 1024 * 1024
NAVIGATE_TIMEOUT_MS = 45_000
FETCH_TIMEOUT = 25.0
TITLE_PATTERN = re.compile(rb"<title[^>]*>(.{0,2048}?)</title>", re.IGNORECASE | re.DOTALL)


def owned_path(value):
    text(value, 4096, 1)
    path = Path(value)
    require(path.is_absolute() and not any(ord(c) < 32 for c in value), "Expected owned absolute path")
    return path


def relay_endpoint(value):
    text(value, 2048, 1)
    require(not any(c in value for c in "\r\n\t "), "Invalid relay endpoint")
    try:
        url = urlsplit(value)
    except ValueError as exc:
        raise Fault("invalid_request", "Invalid relay endpoint") from exc
    require(url.scheme == "http" and url.hostname == "127.0.0.1" and url.port is not None
            and not url.username and not url.password and not url.query and not url.fragment
            and url.path in ("", "/"), "Expected loopback relay endpoint")
    return value


def engine_url(value, fixture):
    """Host-side policy is authoritative; this is the child-side copy."""
    text(value, 4096, 1)
    require(not any(c in value for c in "\\\r\n\t") and not any(ord(c) < 32 for c in value),
            "Invalid destination")
    try:
        url = urlsplit(value)
    except ValueError as exc:
        raise Fault("invalid_request", "Invalid destination") from exc
    require(url.scheme in ("http", "https") and url.hostname and not url.username and not url.password,
            "Expected a plain web address")
    host = url.hostname.rstrip(".")
    require("higgsfield" not in host.lower(), "Destination is frozen", "destination_frozen", 403)
    if fixture:
        require(url.scheme == "http" and host == "127.0.0.1",
                "Fixture mode reaches local fixture pages only")
    else:
        require(not host.endswith(".local") and host not in ("localhost",) and not host.endswith(".localhost")
                and not host.endswith(".internal"), "Local names are not web destinations")
        require(url.port is None or url.port in (80, 443), "Only web ports are reachable")
    return value


def extract_title(html):
    if isinstance(html, str):
        html = html.encode("utf-8", "ignore")
    match = TITLE_PATTERN.search(html)
    if not match:
        return ""
    title = re.sub(rb"<[^>]+>", b"", match.group(1))
    for entity, byte in ((b"&amp;", b"&"), (b"&lt;", b"<"), (b"&gt;", b">"), (b"&quot;", b'"'), (b"&#39;", b"'")):
        title = title.replace(entity, byte)
    text_value = title.decode("utf-8", "ignore").strip()
    return text_value[:500]


class Engine:
    def __init__(self):
        self.config = None
        self.session = None
        self.shutdown = False

    def descriptor(self):
        try:
            version = importlib.metadata.version("scrapling")
        except Exception:
            version = ""
        return {"protocol": PROTOCOL, "scrapling": version, "fixtureMode": bool(self.config and self.config["fixture"]),
                "capabilities": {"navigate": True, "fetch": True, "interactive": False}}

    def proxy_fields(self):
        proxy = self.config["proxy"]
        if proxy is None:
            return None, None
        split = urlsplit(proxy["server"])
        credentials = f"{proxy['username']}:{proxy['password']}"
        return proxy, f"http://{credentials}@{split.hostname}:{split.port}"

    async def ensure_session(self):
        if self.session is not None:
            return self.session
        from scrapling.fetchers import AsyncStealthySession
        proxy, _ = self.proxy_fields()
        settings = {"max_pages": 2, "headless": True, "executable_path": str(self.config["browsers"]),
                    "user_data_dir": str(self.config["profile"] / "chromium"),
                    "block_webrtc": True, "hide_canvas": True, "retries": 1,
                    # Cloudflare solving waits on real challenge pages only; it is
                    # meaningless against local fixtures and slows tests down.
                    "solve_cloudflare": self.config["proxy"] is not None}
        if proxy is not None:
            settings["proxy"] = proxy
        session = AsyncStealthySession(**settings)
        await session.start()
        self.session = session
        return session

    async def navigate(self, url):
        session = await self.ensure_session()
        try:
            page = await session.fetch(url, timeout=NAVIGATE_TIMEOUT_MS, network_idle=True)
        except Exception:
            raise Fault("engine_navigation_failed", "The page could not be fetched by the Scrapling engine.", 502)
        html = page.html_content or ""
        truncated = len(html) > MAX_BODY
        if isinstance(html, str):
            html = html[:MAX_BODY]
        return {"status": page.status, "url": text(str(page.url), 2048, 0), "title": extract_title(html),
                "html": html, "encoding": text(getattr(page, "encoding", "") or "", 64, 0),
                "engineMode": "stealthy", "truncated": truncated, "interactive": False}

    async def fetch(self, url):
        from scrapling.fetchers import AsyncFetcher
        _, proxy_url = self.proxy_fields()
        settings = {"timeout": FETCH_TIMEOUT, "retries": 0}
        if proxy_url is not None:
            settings["proxy"] = proxy_url
        try:
            page = await AsyncFetcher.get(url, **settings)
        except Exception:
            raise Fault("engine_fetch_failed", "The address could not be fetched by the Scrapling engine.", 502)
        body = page.html_content or ""
        truncated = len(body) > MAX_BODY
        headers = page.headers or {}
        content_type = ""
        for key, value in headers.items():
            if str(key).lower() == "content-type":
                content_type = text(str(value), 200, 0)
                break
        return {"status": page.status, "url": text(str(page.url), 2048, 0), "contentType": content_type,
                "body": body[:MAX_BODY], "truncated": truncated}

    async def dispatch(self, frame):
        require(isinstance(frame, dict), "Expected control frame")
        uid(frame.get("id"))
        op = frame.get("op")
        if op == "bootstrap":
            obj(frame, ("id", "op", "browsersExecutable", "userDataDir"),
                ("proxyServer", "proxyUsername", "proxyPassword", "fixtureRoot"))
            require(self.config is None, "Engine already bootstrapped", "already_bootstrapped", 409)
            browsers = owned_path(frame["browsersExecutable"])
            require(browsers.exists() and not browsers.is_symlink()
                    and (os.stat(browsers).st_mode & 0o111), "Bundled browser shell unavailable",
                    "browser_unavailable", 503)
            profile = owned_path(frame["userDataDir"])
            profile.mkdir(parents=True, exist_ok=True)
            require(not profile.is_symlink() and profile.is_dir() and profile.stat().st_uid == os.getuid(),
                    "Invalid engine profile directory", "profile_unavailable", 503)
            os.chmod(profile, 0o700)
            fixture = frame.get("fixtureRoot")
            if fixture is not None:
                fixture = owned_path(fixture)
                require(fixture.is_dir() and not fixture.is_symlink(), "Invalid fixture root",
                        "fixture_unavailable", 503)
            proxy = None
            if "proxyServer" in frame or "proxyUsername" in frame or "proxyPassword" in frame:
                obj(frame, ("id", "op", "browsersExecutable", "userDataDir", "proxyServer",
                            "proxyUsername", "proxyPassword"), ("fixtureRoot",))
                username = text(frame["proxyUsername"], 128, 1)
                password = text(frame["proxyPassword"], 256, 1)
                require(not any(c in username + password for c in "\r\n\t"), "Invalid relay credential")
                proxy = {"server": relay_endpoint(frame["proxyServer"]), "username": username, "password": password}
            require(proxy is not None or fixture is not None,
                    "The engine requires the native relay", "relay_required", 503)
            self.config = {"browsers": browsers, "profile": profile, "proxy": proxy, "fixture": fixture is not None}
            return self.descriptor()
        require(self.config is not None and not self.shutdown, "Engine not bootstrapped", "not_bootstrapped", 409)
        if op == "navigate":
            obj(frame, ("id", "op", "url"))
            return await self.navigate(engine_url(frame["url"], self.config["fixture"]))
        if op == "fetch":
            obj(frame, ("id", "op", "url"))
            return await self.fetch(engine_url(frame["url"], self.config["fixture"]))
        if op == "close":
            await self.aclose()
            self.shutdown = True
            return {"stopped": True}
        raise Fault("Unknown engine operation")

    async def aclose(self):
        session, self.session = self.session, None
        if session is not None:
            try:
                await asyncio.wait_for(session.close(), 10)
            except Exception:
                pass
        self.config = None


async def respond(engine, raw):
    identity = None
    try:
        frame = decode(raw)
        if isinstance(frame, dict):
            identity = uid(frame.get("id"))
        result = await engine.dispatch(frame)
        value = {"id": identity, "ok": True, "result": result}
    except Fault as exc:
        value = {"id": identity, "ok": False, "error": exc.wire()["error"]}
    except Exception:
        value = {"id": identity, "ok": False,
                 "error": Fault("engine_failed", "Engine operation failed", 503).wire()["error"]}
    payload = canonical(value).encode() + b"\n"
    require(len(payload) <= MAX_RESPONSE, "Control response exceeds bound", "response_limit", 500)
    return payload


async def write_all(payload):
    view = memoryview(payload)
    deadline = asyncio.get_running_loop().time() + 60
    while view:
        require(asyncio.get_running_loop().time() < deadline, "Control response timed out")
        try:
            written = os.write(1, view[:65536])
        except BlockingIOError:
            await asyncio.sleep(0.05)
            continue
        require(written > 0, "Control pipe closed")
        view = view[written:]


async def main():
    require(sys.platform in ("darwin", "linux"), "Engine containment unavailable", "platform_unavailable", 503)
    os.umask(0o077)
    loop = asyncio.get_running_loop()
    engine = Engine()
    stopping = asyncio.Event()
    for received in (signal.SIGTERM, signal.SIGINT):
        loop.add_signal_handler(received, stopping.set)
    lines = asyncio.Queue(maxsize=8)

    def reader():
        pending = bytearray()
        try:
            while True:
                chunk = os.read(0, 65536)
                if not chunk:
                    loop.call_soon_threadsafe(lines.put_nowait, None)
                    return
                pending.extend(chunk)
                while b"\n" in pending:
                    end = pending.index(b"\n")
                    line = bytes(pending[:end])
                    del pending[:end + 1]
                    loop.call_soon_threadsafe(lines.put_nowait, line)
        except Exception:
            loop.call_soon_threadsafe(lines.put_nowait, None)

    threading.Thread(target=reader, daemon=True).start()
    try:
        while not stopping.is_set() and not engine.shutdown:
            incoming = asyncio.create_task(lines.get())
            halted = asyncio.create_task(stopping.wait())
            done, _ = await asyncio.wait({incoming, halted}, return_when=asyncio.FIRST_COMPLETED)
            halted.cancel()
            if incoming not in done:
                incoming.cancel()
                break
            line = incoming.result()
            if line is None:
                break
            require(len(line) <= MAX_REQUEST, "Control frame exceeds bound")
            if line:
                await write_all(await respond(engine, line))
    finally:
        await engine.aclose()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except Exception:
        # A fatal error tears down the owned process without leaking any
        # configuration, destination, or secret text on stdout or stderr.
        sys.exit(1)
