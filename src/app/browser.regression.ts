import { strict as assert } from 'node:assert';
import { readFileSync } from 'node:fs';
import { acceptsBrowserRevision, browserActionFailure, type OwnedBrowserStatus } from './browser-contracts.ts';

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
  const status: OwnedBrowserStatus = {
    revision: 4, available: true, phase: 'closed', engine: 'webkit',
    availableEngines: [{ id: 'webkit', label: 'WebKit' }],
    chromiumUnavailableReason: 'Chromium is unavailable until startup protection is proven.',
    persistent: false, profileId: null, url: null, service: null, error: null, automationReady: false,
  };
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
  return ['browser revisions reject stale and invalid snapshots', 'late open response cannot overwrite a newer close event', 'unavailable and native error snapshots never report action success', 'browser spacing, colors, and composed-input borders follow the design contract'];
}
