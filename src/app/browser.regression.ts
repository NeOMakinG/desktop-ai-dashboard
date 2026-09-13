import { strict as assert } from 'node:assert';
import { readFileSync } from 'node:fs';
import { acceptsBrowserRevision, browserActionFailure, defaultBrowserEngine, realChromiumStatusLine, type AvailableBrowserEngine, type OwnedBrowserStatus, type RealChromiumStatus } from './browser-contracts.ts';

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
    supported: true, installed: false, phase: 'idle', progressPercent: null,
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
