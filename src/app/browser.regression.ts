import { strict as assert } from 'node:assert';
import { readFileSync } from 'node:fs';
import {
  acceptsBrowserRevision, browserActionFailure, cdpModifiers, cdpMouseButton, defaultBrowserEngine,
  embeddedContainRect, embeddedPointToPage, embeddedShortcut, embeddedTabLabel, isSecureUrl,
  normalizeAddress, realChromiumBinaryLabel, realChromiumStatusLine,
  type AvailableBrowserEngine, type OwnedBrowserStatus, type RealChromiumStatus,
} from './browser-contracts.ts';

export function runBrowserRegressionChecks() {
  assert.equal(acceptsBrowserRevision(-1, 0), true);
  assert.equal(acceptsBrowserRevision(7, 8), true);
  assert.equal(acceptsBrowserRevision(7, 7), true);
  assert.equal(acceptsBrowserRevision(8, 7), false);
  for (const invalid of [NaN, Infinity, -1, 1.5, Number.MAX_SAFE_INTEGER + 1]) {
    assert.equal(acceptsBrowserRevision(0, invalid), false);
  }
  let state = { revision: 1, phase: 'opening' };
  const apply = (next: typeof state) => { if (acceptsBrowserRevision(state.revision, next.revision)) state = next; };
  apply({ revision: 3, phase: 'closed' });
  apply({ revision: 2, phase: 'open' });
  assert.deepEqual(state, { revision: 3, phase: 'closed' });
  const idleReal: RealChromiumStatus = {
    supported: true, installed: false, binary: null, phase: 'idle', progressPercent: null,
    pid: null, cdpReady: false, error: null, version: '151.0.7922.34',
  };
  const status: OwnedBrowserStatus = {
    revision: 4, available: true, phase: 'closed', engine: 'webkit',
    availableEngines: [{ id: 'webkit', label: 'WebKit' }],
    chromiumUnavailableReason: null,
    realChromium: idleReal,
    persistent: false, profileId: null, url: null, service: null, error: null, automationReady: false,
    interactive: false, navigating: false,
  };
  // Scrapling is the advertised default when its sealed engine resources
  // verify; without them the default stays WebKit — never a silent swap.
  const enginesWith: AvailableBrowserEngine[] = [
    { id: 'scrapling', label: 'Scrapling (Chromium snapshots)' },
    { id: 'webkit', label: 'WebKit' },
  ];
  assert.equal(defaultBrowserEngine(enginesWith), 'scrapling');
  assert.equal(defaultBrowserEngine([{ id: 'webkit', label: 'WebKit' }]), 'webkit');
  assert.equal(defaultBrowserEngine([]), 'unavailable');
  // Installed real Chrome advertises first and therefore becomes the default.
  assert.equal(defaultBrowserEngine([
    { id: 'chromium', label: 'Chrome (full browser)' },
    ...enginesWith,
  ]), 'chromium');
  // Real-browser card lines stay honest for every lifecycle phase.
  assert.equal(realChromiumStatusLine(idleReal), 'Chrome 151.0.7922.34 downloads on first use (about 190 MB).');
  assert.equal(realChromiumStatusLine({ ...idleReal, installed: true }), 'Chrome is installed and ready to open.');
  assert.equal(realChromiumStatusLine({ ...idleReal, phase: 'downloading', progressPercent: 37 }), 'Downloading Chrome 151.0.7922.34… 37%');
  assert.equal(realChromiumStatusLine({ ...idleReal, phase: 'running', pid: 4242, cdpReady: true }), 'Chrome is running (pid 4242) · assistant link ready');
  assert.equal(realChromiumStatusLine({ ...idleReal, phase: 'running', pid: null, cdpReady: false }), 'Chrome is running');
  assert.equal(realChromiumStatusLine({ ...idleReal, phase: 'exited' }), 'Chrome was closed. Open it again any time — your sign-ins are kept.');
  assert.equal(realChromiumStatusLine({ ...idleReal, phase: 'error', error: 'The Chrome download could not be completed. Check your connection and try again.' }), 'The Chrome download could not be completed. Check your connection and try again.');
  assert.equal(realChromiumStatusLine({ ...idleReal, phase: 'error' }), 'Chrome could not be prepared.');
  // Stealth addendum: the binary in use is labeled honestly — the user's own
  // Google Chrome preferred, the bundled Chrome for Testing as fallback.
  assert.equal(realChromiumStatusLine({ ...idleReal, installed: true, binary: 'system_chrome' }), 'Your installed Google Chrome is ready to open inside Forma.');
  assert.equal(realChromiumBinaryLabel('system_chrome'), 'Using your installed Google Chrome (with a separate Forma profile).');
  assert.equal(realChromiumBinaryLabel('chrome_for_testing'), 'Using the bundled Chrome for Testing build.');
  assert.equal(realChromiumBinaryLabel(null), null);

  // Embedded surface geometry: contain-fit letterboxes and pointer mapping
  // returns page CSS coordinates (or null inside the letterbox bars).
  const fit = embeddedContainRect(1000, 500, 1600, 1000);
  assert.deepEqual(fit, { x: 100, y: 0, width: 800, height: 500 });
  assert.deepEqual(embeddedContainRect(0, 500, 1600, 1000), { x: 0, y: 0, width: 0, height: 0 });
  const meta = { deviceWidth: 1600, deviceHeight: 1000 };
  assert.deepEqual(embeddedPointToPage(100, 0, fit, meta), { x: 0, y: 0 });
  assert.deepEqual(embeddedPointToPage(900, 500, fit, meta), { x: 1600, y: 1000 });
  assert.deepEqual(embeddedPointToPage(500, 250, fit, meta), { x: 800, y: 500 });
  assert.equal(embeddedPointToPage(50, 250, fit, meta), null, 'letterbox clicks are never forwarded');
  assert.equal(embeddedPointToPage(10, 10, { x: 0, y: 0, width: 0, height: 0 }, meta), null);
  // CDP modifier bitmask (Alt=1 Ctrl=2 Meta=4 Shift=8) and button names.
  assert.equal(cdpModifiers({ altKey: true, ctrlKey: false, metaKey: true, shiftKey: true }), 13);
  assert.equal(cdpModifiers({ altKey: false, ctrlKey: false, metaKey: false, shiftKey: false }), 0);
  assert.equal(cdpMouseButton(0), 'left');
  assert.equal(cdpMouseButton(2), 'right');
  assert.equal(cdpMouseButton(4), null);
  // App-safe shortcuts only while the canvas is focused; plain keys forward.
  assert.equal(embeddedShortcut({ key: 'T', metaKey: true, ctrlKey: false, altKey: false, shiftKey: false }), 'new_tab');
  assert.equal(embeddedShortcut({ key: 'w', metaKey: true, ctrlKey: false, altKey: false, shiftKey: false }), 'close_tab');
  assert.equal(embeddedShortcut({ key: 'v', metaKey: true, ctrlKey: false, altKey: false, shiftKey: false }), 'paste');
  assert.equal(embeddedShortcut({ key: 't', metaKey: false, ctrlKey: false, altKey: false, shiftKey: false }), null);
  assert.equal(embeddedShortcut({ key: 't', metaKey: true, ctrlKey: false, altKey: false, shiftKey: true }), null);
  // Address normalization: https passes, bare domains upgrade, anything that
  // needs rewriting beyond that is refused rather than guessed.
  assert.equal(normalizeAddress(' https://example.com/a '), 'https://example.com/a');
  assert.equal(normalizeAddress('example.com'), 'https://example.com');
  assert.equal(normalizeAddress('http://example.com'), null);
  assert.equal(normalizeAddress('javascript://x.y'), null);
  assert.equal(normalizeAddress('not a url'), null);
  assert.equal(normalizeAddress('word'), null);
  assert.equal(normalizeAddress(''), null);
  assert.equal(isSecureUrl('https://example.com/'), true);
  assert.equal(isSecureUrl('about:blank'), false);
  assert.equal(isSecureUrl(null), false);
  assert.equal(embeddedTabLabel({ title: ' Example ', url: 'https://example.com/' }), 'Example');
  assert.equal(embeddedTabLabel({ title: '', url: 'https://example.com/path' }), 'example.com');
  assert.equal(embeddedTabLabel({ title: '', url: '' }), 'New tab');
  assert.equal(browserActionFailure({ ...status, availableEngines: enginesWith, engine: 'scrapling', phase: 'open', interactive: false, navigating: true }), null);
  assert.equal(browserActionFailure({ ...status, engine: 'scrapling', phase: 'error', error: 'The page could not be fetched by the Scrapling engine.' }), 'The page could not be fetched by the Scrapling engine.');
  assert.equal(browserActionFailure(status), null);
  assert.equal(browserActionFailure({ ...status, phase: 'opening' }), null);
  assert.equal(browserActionFailure({ ...status, phase: 'error', error: 'Native gate failed.' }), 'Native gate failed.');
  assert.equal(browserActionFailure({ ...status, phase: 'unavailable', available: false }), 'The owned browser is not available on this build.');
  assert.equal(browserActionFailure({ ...status, available: false, error: 'Unsupported platform.' }), 'Unsupported platform.');
  const css = readFileSync(new URL('./browser.css', import.meta.url), 'utf8');
  const spacing = new Set([0, 2, 4, 8, 12, 16, 24, 32, 48, 64]);
  for (const declaration of css.matchAll(/(?:^|[;{])\s*(?:gap|row-gap|column-gap|padding(?:-[a-z-]+)?|margin(?:-[a-z-]+)?|outline-offset)\s*:\s*([^;}]+)/g)) {
    for (const value of declaration[1].matchAll(/(-?\d+(?:\.\d+)?)px/g)) {
      assert.ok(spacing.has(Number(value[1])), `Off-scale browser spacing: ${declaration[0]}`);
    }
  }
  assert.doesNotMatch(css, /#[0-9a-f]{3,8}\b|\b(?:rgba?|hsla?)\(/i);
  assert.match(css, /\.browser-page\s*\{\s*padding\s*:\s*24px\s+24px\s+32px\s*;/);
  const address = css.match(/\.browser-address\s*\{([^}]+)\}/)?.[1] || '';
  assert.match(address, /border-radius\s*:\s*8px\s*;/);
  const input = css.match(/\.browser-page\s+\.browser-address\s+input\s*\{([^}]+)\}/)?.[1] || '';
  assert.match(input, /border\s*:\s*0\s*;/);
  assert.match(input, /background\s*:\s*transparent\s*;/);
  const browserView = readFileSync(new URL('./BrowserView.tsx', import.meta.url), 'utf8');
  const websiteActions = browserView.slice(browserView.indexOf('export function BrowserConnections('), browserView.indexOf('export function BrowserView('));
  const engineStates = browserView.slice(browserView.indexOf('export function BrowserView('));
  assert.match(engineStates, /defaultBrowserEngine\(engines\)/);
  assert.match(engineStates, /aria-checked=\{engineChoice === engine\.id\}/);
  // The real-browser card is honest: install/run status line, drive toggle
  // default-off wording, and the founder's sign-in note.
  assert.match(engineStates, /realChromiumStatusLine\(real\)/);
  assert.match(engineStates, /Sign in to your accounts here; the assistant can drive this browser when you ask\./);
  assert.match(engineStates, /Assistant may drive the browser/);
  assert.match(engineStates, /Off by default\. When on, the assistant can control this Chrome while it is open\./);
  assert.match(engineStates, /checked=\{assistantDrive\}/);
  assert.match(engineStates, /Fetching the page through the Scrapling engine…/);
  assert.match(engineStates, /Read-only snapshots: pages are fetched by Scrapling's engine and shown without live scripts\. Typing and signing in inside this window are not supported yet\./);
  assert.match(engineStates, /browser\.status\.navigating && open/);
  // Embedded in-app Chrome: rendered while the real browser runs, replacing
  // the generic toolbar/home card with Forma's own tab strip + canvas.
  assert.match(engineStates, /const embeddedActive = realSelected && realRunning;/);
  // Full-panel edge-to-edge embedded browser (founder directive): when active
  // it returns early with no hero header and the embedded surface fills the panel.
  assert.match(engineStates, /if \(embeddedActive\) \{/);
  assert.match(engineStates, /className="browser-page embedded-full"/);
  assert.match(engineStates, /<EmbeddedChrome onError=\{setEmbeddedError\} assistantDrive=\{assistantDrive\} onAssistantDrive=\{onAssistantDrive\} \/>/);
  assert.match(engineStates, /realChromiumBinaryLabel\(real\.binary\)/);
  const embedded = readFileSync(new URL('./EmbeddedChrome.tsx', import.meta.url), 'utf8');
  assert.match(embedded, /role="tablist" aria-label="Browser tabs"/);
  // Auto-reconnect UX: a slim reconnecting note, never a hard error, on a
  // transient CDP drop while the browser is still running.
  assert.match(embedded, /state\.running && !state\.connected/);
  assert.match(embedded, /Reconnecting to Chrome…/);
  assert.match(embedded, /Assistant drive/);
  assert.match(embedded, /browser_embedded_new_tab/);
  assert.match(embedded, /browser_embedded_close_tab/);
  assert.match(embedded, /browser_embedded_navigate/);
  assert.match(embedded, /browser_embedded_input/);
  assert.match(embedded, /browser_embedded_pop/);
  assert.match(embedded, /embedded-browser:frame/);
  assert.match(embedded, /Page\.screencastFrame/, 'frame source documented');
  assert.match(embedded, /Click to interact/, 'honest focus state before keys forward');
  assert.match(embedded, /Chrome is popped out into its own window\./);
  assert.doesNotMatch(embedded, /dangerouslySetInnerHTML|eval\(/);
  // The embedded surface never bypasses the host URL gate from the renderer.
  assert.match(embedded, /normalizeAddress\(address\)/);
  const cdpNative = readFileSync(new URL('../../src-tauri/src/browser/cdp.rs', import.meta.url), 'utf8');
  assert.match(cdpNative, /isTrusted/, 'trusted-input posture documented at the source');
  assert.match(cdpNative, /Message::Ping/, 'server pings are answered so the socket is not dropped');
  const browserNative = readFileSync(new URL('../../src-tauri/src/browser.rs', import.meta.url), 'utf8');
  assert.match(browserNative, /Page\.screencastFrameAck/, 'every frame is acked so the stream continues');
  assert.match(browserNative, /reconnecting_payload/, 'transient CDP drops reconnect instead of erroring');
  const chromiumNative = readFileSync(new URL('../../src-tauri/src/browser/chromium.rs', import.meta.url), 'utf8');
  assert.match(chromiumNative, /--window-position=20000,20000/);
  // Launch surface: build_argv only ever pushes the whitelisted flags — no
  // --enable-automation, --headless, --load-extension, or proxy flags.
  const argvBody = chromiumNative.slice(chromiumNative.indexOf('let mut argv = vec!['), chromiumNative.indexOf('pub(super) fn parse_devtools_port'));
  assert.ok(argvBody.length > 0);
  assert.doesNotMatch(argvBody, /"--(?!user-data-dir|remote-debugging-port|no-first-run|no-default-browser-check|window-position|window-size)/);
  assert.match(websiteActions, /aria-label="Website browsing"/);
  assert.match(websiteActions, /These websites do not give the agent Gmail or Calendar access/);
  assert.match(websiteActions, /onClick=\{\(\) => open\(service\.id\)\}>Open website/);
  assert.doesNotMatch(websiteActions, />Sign in /);
  const settings = readFileSync(new URL('./Settings.tsx', import.meta.url), 'utf8');
  const accounts = settings.slice(settings.indexOf('export function Accounts('), settings.indexOf('function ProviderSetup('));
  assert.doesNotMatch(accounts, /browser\.(?:open|busy|status)|authorizeUrl/);
  assert.match(accounts, /const disabled = !browser\.native \|\| connectors\.busy \|\| !capabilities\.googleAvailable;/);
  assert.match(accounts, /Google sign-in opens your default browser, not the Forma browser/);
  assert.match(accounts, /never reads or copies that browser’s cookies or profiles/);
  const connectors = readFileSync(new URL('./connectors.ts', import.meta.url), 'utf8');
  assert.match(connectors, /type StartResponse = \{ attemptId: string \};/);
  assert.doesNotMatch(connectors, /authorizeUrl|window\.open|browser\.open/);
  assert.match(connectors, /invoke<StartResponse>\('connectors_start_google', \{ scopes: \[\.\.\.scopes\] \}\)/);
  const native = readFileSync(new URL('../../src-tauri/src/connectors.rs', import.meta.url), 'utf8');
  const start = native.slice(native.indexOf('pub async fn connectors_start_google('), native.indexOf('pub async fn connectors_refresh('));
  assert.match(start, /crate::first_party\(&window\)\?/);
  assert.match(start, /oauth::authorize_url\(client_id, &redirect, &scopes, &state, &pkce\.challenge\)/);
  assert.match(start, /launch_google_browser\(&core, &id, loopback, &authorize_url, &cancel, opener::spawn\)/);
  assert.match(start, /Ok\(StartResponse \{ attempt_id \}\)/);
  assert.doesNotMatch(start, /(?:println!|dbg!|log::|tracing::)/);
  return ['browser revisions reject stale and invalid snapshots', 'late open response cannot overwrite a newer close event', 'unavailable and native error snapshots never report action success', 'browser spacing, colors, and composed-input borders follow the design contract', 'Google OAuth uses a native-owned system-browser route with no embedded-browser dependency or renderer authorization URL', 'scrapling engine default and read-only snapshot states stay explicit and honest'];
}
