"""Assistant drive path over the user's real Forma browser (CDP).

v1 primitives — navigate / snapshot / click / type — used by the managed
Hermes runtime after `forma_browser_session` hands over a loopback CDP
endpoint (only when the user's default-off "assistant may drive the browser"
preference is on and the real browser is running).

Every interaction is deliberately human-paced: a randomized pause before each
action, a randomized mouse-button hold on clicks, and randomized per-keystroke
delays while typing. The browser is the user's own headful Chrome on the
user's own IP; the assistant must look like a hand on the same keyboard, not
a flood of synthetic events. This pacing is the anti-ban layer — do not
remove it to make automation "faster".

No secrets ever reach this module: the CDP endpoint is not a credential, and
page content returned to the model is bounded plain text. Connections are
detached with `close()`, never by killing the user's browser.
"""
from __future__ import annotations

import asyncio
import json
import random
import sys
from pathlib import Path
from urllib.parse import urlsplit

if __package__ in (None, ""):
    sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

from forma_runtime.contracts import Fault, require, text

MAX_SNAPSHOT_TEXT = 200_000
MAX_TYPED_TEXT = 2_000
MAX_SELECTOR = 500
NAVIGATE_TIMEOUT_MS = 45_000
ACTION_TIMEOUT_MS = 15_000
# Human pacing bounds. Randomized per action; see module docstring.
PACING_PAUSE_SECONDS = (0.25, 0.9)
KEYSTROKE_DELAY_MS = (40, 140)
CLICK_HOLD_MS = (40, 120)


def cdp_endpoint(value):
    """The drive path only ever attaches to the loopback endpoint the host
    handed out. Anything else — remote hosts, TLS, credentials, paths — is
    rejected before any connection is attempted."""
    text(value, 200, 1)
    require(not any(ord(c) < 33 for c in value), "Invalid CDP endpoint")
    try:
        url = urlsplit(value)
    except ValueError as exc:
        raise Fault("invalid_request", "Invalid CDP endpoint") from exc
    require(url.scheme == "http" and url.hostname == "127.0.0.1" and url.port is not None
            and not url.username and not url.password and not url.query and not url.fragment
            and url.path in ("", "/"), "Expected loopback CDP endpoint")
    return f"http://127.0.0.1:{url.port}"


def drive_url(value, allow_local=False):
    """Destination policy for assistant-initiated navigation. The frozen
    provider stays frozen here too; local/internal names are denied except in
    the explicit test fixture mode (loopback http only)."""
    text(value, 4096, 1)
    require(not any(ord(c) < 33 for c in value) and "\\" not in value, "Invalid destination")
    try:
        url = urlsplit(value)
    except ValueError as exc:
        raise Fault("invalid_request", "Invalid destination") from exc
    require(url.scheme in ("http", "https") and url.hostname
            and not url.username and not url.password, "Expected a plain web address")
    host = url.hostname.rstrip(".").lower()
    require("higgsfield" not in host, "Destination is frozen", "destination_frozen", 403)
    if allow_local:
        require(url.scheme == "http" and host == "127.0.0.1",
                "Local mode reaches loopback fixtures only")
    else:
        require(host not in ("localhost", "127.0.0.1", "0.0.0.0", "::1")
                and not host.endswith(".localhost") and not host.endswith(".local")
                and not host.endswith(".internal") and not host.startswith("127.")
                and not host.startswith("10.") and not host.startswith("192.168.")
                and not host.startswith("169.254."), "Local names are not web destinations")
        require(url.port is None or url.port in (80, 443), "Only web ports are reachable")
    return value


def valid_selector(value):
    text(value, MAX_SELECTOR, 1)
    require("\n" not in value and "\r" not in value, "Invalid selector")
    return value


def keystroke_delay_ms():
    return random.randint(*KEYSTROKE_DELAY_MS)


def click_hold_ms():
    return random.randint(*CLICK_HOLD_MS)


async def human_pause():
    """Randomized think-time before every action (anti-ban pacing)."""
    await asyncio.sleep(random.uniform(*PACING_PAUSE_SECONDS))


class BrowserDrive:
    """Async v1 toolset over one attached CDP browser.

    Attaches to the existing browser (never launches one) and works in the
    user's current context/tab, so the assistant sees exactly the sessions
    the user signed into.
    """

    def __init__(self, endpoint, allow_local=False):
        self.endpoint = cdp_endpoint(endpoint)
        self.allow_local = bool(allow_local)
        self._playwright = None
        self._browser = None

    async def __aenter__(self):
        return await self.start()

    async def __aexit__(self, *_exc):
        await self.close()

    async def start(self):
        from patchright.async_api import async_playwright
        self._playwright = await async_playwright().start()
        try:
            self._browser = await self._playwright.chromium.connect_over_cdp(
                self.endpoint, timeout=ACTION_TIMEOUT_MS)
        except Exception as exc:
            await self._playwright.stop()
            self._playwright = None
            raise Fault("browser_unavailable", "The browser CDP endpoint did not answer", 503) from exc
        return self

    async def close(self):
        """Detach only. The user's browser stays running with all its tabs."""
        browser, self._browser = self._browser, None
        playwright, self._playwright = self._playwright, None
        try:
            if browser is not None:
                await browser.close()
        finally:
            if playwright is not None:
                await playwright.stop()

    async def _page(self):
        require(self._browser is not None, "Drive session not started", "not_started", 409)
        contexts = self._browser.contexts
        context = contexts[0] if contexts else await self._browser.new_context()
        pages = context.pages
        return pages[-1] if pages else await context.new_page()

    async def _location(self, page):
        try:
            title = (await page.title() or "")[:500]
        except Exception:
            title = ""
        return {"url": text(str(page.url)[:2048], 2048, 0), "title": title}

    async def navigate(self, url):
        url = drive_url(url, self.allow_local)
        page = await self._page()
        await human_pause()
        try:
            response = await page.goto(url, timeout=NAVIGATE_TIMEOUT_MS,
                                       wait_until="domcontentloaded")
        except Exception as exc:
            raise Fault("drive_navigation_failed", "The page could not be opened", 502) from exc
        result = await self._location(page)
        result["status"] = response.status if response is not None else None
        return result

    async def snapshot(self):
        """Bounded plain-text view of the current page — never raw HTML with
        inline secrets/scripts, never a screenshot of other tabs."""
        page = await self._page()
        try:
            content = await page.inner_text("body", timeout=ACTION_TIMEOUT_MS)
        except Exception:
            content = ""
        truncated = len(content) > MAX_SNAPSHOT_TEXT
        result = await self._location(page)
        result.update(text=content[:MAX_SNAPSHOT_TEXT], truncated=truncated)
        return result

    async def click(self, selector):
        selector = valid_selector(selector)
        page = await self._page()
        await human_pause()
        try:
            locator = page.locator(selector).first
            await locator.hover(timeout=ACTION_TIMEOUT_MS)
            await asyncio.sleep(random.uniform(0.05, 0.2))
            await locator.click(timeout=ACTION_TIMEOUT_MS, delay=click_hold_ms())
        except Exception as exc:
            raise Fault("drive_action_failed", "The element could not be clicked", 502) from exc
        result = await self._location(page)
        result["clicked"] = selector
        return result

    async def type(self, selector, value):
        selector = valid_selector(selector)
        text(value, MAX_TYPED_TEXT, 0)
        page = await self._page()
        await human_pause()
        try:
            locator = page.locator(selector).first
            await locator.click(timeout=ACTION_TIMEOUT_MS, delay=click_hold_ms())
            # Per-keystroke randomized delay: human typing cadence.
            await locator.press_sequentially(value, timeout=NAVIGATE_TIMEOUT_MS,
                                             delay=keystroke_delay_ms())
        except Exception as exc:
            raise Fault("drive_action_failed", "The field could not be typed into", 502) from exc
        result = await self._location(page)
        result["typed"] = len(value)
        return result


async def _run(endpoint, op, arguments, allow_local):
    async with BrowserDrive(endpoint, allow_local=allow_local) as drive:
        if op == "navigate" and len(arguments) == 1:
            return await drive.navigate(arguments[0])
        if op == "snapshot" and not arguments:
            return await drive.snapshot()
        if op == "click" and len(arguments) == 1:
            return await drive.click(arguments[0])
        if op == "type" and len(arguments) == 2:
            return await drive.type(arguments[0], arguments[1])
        raise Fault("invalid_request", "Usage: drive.py <endpoint> navigate <url> | snapshot | click <selector> | type <selector> <text>")


def main(argv):
    allow_local = "--allow-local" in argv
    arguments = [value for value in argv if value != "--allow-local"]
    if len(arguments) < 2:
        raise Fault("invalid_request", "Usage: drive.py <endpoint> <op> [args…]")
    result = asyncio.run(_run(arguments[0], arguments[1], arguments[2:], allow_local))
    print(json.dumps(result, ensure_ascii=False))


if __name__ == "__main__":
    try:
        main(sys.argv[1:])
    except Fault as fault:
        print(json.dumps(fault.wire(), ensure_ascii=False))
        sys.exit(1)
