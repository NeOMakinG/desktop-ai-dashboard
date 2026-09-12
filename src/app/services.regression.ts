import { strict as assert } from 'node:assert';
import { existsSync, readFileSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { createElement, type ReactElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { transformWithOxc } from 'vite';
import {
  CATALOG_RENDER_CAP,
  CATALOG_STEP,
  catalogCountLine,
  chipsFromCategories,
  connectCardPhase,
  mergeServicePrompt,
  promptsForWorkspace,
  validComposioKey,
  type CatalogSurface,
  type ComposioService,
} from './services-contracts.ts';

function fixtureService(index: number): ComposioService {
  const names = ['GitHub', 'Notion', 'Linear', 'Slack', 'Figma'];
  return {
    slug: `service-${index}`,
    name: names[index % names.length],
    description: 'Synthetic service',
    logo: null,
    categories: index % 2 ? ['Productivity'] : ['Developer Tools'],
    toolsCount: 12,
    noAuth: false,
  };
}
function fixtureSurface(overrides: Partial<CatalogSurface> = {}): CatalogSurface {
  return {
    loading: false,
    error: null,
    items: Array.from({ length: 1500 }, (_, index) => fixtureService(index)),
    totalItems: 1500,
    query: '',
    category: '',
    categories: [],
    connectingService: null,
    connectedSlugs: [],
    onQuery: () => {},
    onCategory: () => {},
    onRetry: () => {},
    onConnect: () => {},
    onCancelConnect: () => {},
    ...overrides,
  };
}

// Node's type stripping does not compile TSX. Use Vite's existing transform in
// memory on first-party source only (no model-code evaluation/build/server).
// Relative .ts imports stay file URLs (Node strips their types); .tsx deps are
// transformed recursively; './media' is stubbed because its JSON module import
// is a Vite feature, not a Node ESM one.
const MEDIA_STUB = 'data:text/javascript,export const media = { cozy: null, coast: null, ambient: null, poster: null };';
const transformed = new Map<string, Promise<string>>();
async function moduleDataUrl(file: string): Promise<string> {
  const cached = transformed.get(file);
  if (cached) return cached;
  const promise = (async () => {
    const source = await readFile(new URL(`./${file}`, import.meta.url), 'utf8');
    let code = (await transformWithOxc(source, file, { jsx: { runtime: 'automatic' } })).code
      .replace(/import ['"]\.\/[^'"]+\.css['"];?\n/g, '');
    const resolutions: Record<string, string> = {};
    for (const match of code.matchAll(/from (['"])(.*?)\1/g)) {
      const specifier = match[2];
      if (Object.hasOwn(resolutions, specifier)) continue;
      if (specifier === './media') { resolutions[specifier] = MEDIA_STUB; continue; }
      if (!specifier.startsWith('.')) { resolutions[specifier] = import.meta.resolve(specifier); continue; }
      const base = specifier.replace(/^\.\//, '');
      const tsx = `./${base}.tsx`;
      if (existsSync(new URL(tsx, import.meta.url))) {
        resolutions[specifier] = await moduleDataUrl(tsx);
      } else {
        resolutions[specifier] = new URL(`./${base}.ts`, import.meta.url).href;
      }
    }
    code = code.replace(/from (['"])(.*?)\1/g, (whole, _quote: string, specifier: string) =>
      Object.hasOwn(resolutions, specifier) ? `from ${JSON.stringify(resolutions[specifier])}` : whole);
    return `data:text/javascript;base64,${Buffer.from(code).toString('base64')}`;
  })();
  transformed.set(file, promise);
  return promise;
}
async function loadTransformed(file: string): Promise<Record<string, unknown>> {
  return import(await moduleDataUrl(file)) as Promise<Record<string, unknown>>;
}

export async function runServicesRegressionChecks() {
  // 1. Token discipline for every new services css file.
  const servicesCss = readFileSync(new URL('./services.css', import.meta.url), 'utf8');
  const spacing = new Set([0, 2, 4, 8, 12, 16, 24, 32, 48, 64]);
  for (const declaration of servicesCss.matchAll(/(?:^|[;{])\s*(?:gap|row-gap|column-gap|padding(?:-[a-z-]+)?|margin(?:-[a-z-]+)?|outline-offset)\s*:\s*([^;}]+)/g)) {
    for (const value of declaration[1].matchAll(/(-?\d+(?:\.\d+)?)px/g)) {
      assert.ok(spacing.has(Number(value[1])), `Off-scale services spacing: ${declaration[0]}`);
    }
  }
  assert.doesNotMatch(servicesCss, /#[0-9a-f]{3,8}\b|\b(?:rgba?|hsla?)\(/i);
  assert.match(servicesCss, /\.service-catalog\s*\{[^}]*max-height:\s*min\(310px,\s*40dvh\)/);
  assert.match(servicesCss, /\.connect-card\s*\{[^}]*border-radius:\s*16px/);
  assert.doesNotMatch(servicesCss, /outline\s*:\s*none/);

  // 2. Catalog bounds: bounded render, honest count, chip ceiling.
  const services = await loadTransformed('services.tsx');
  const ServiceResults = services.ServiceResults as (props: { surface: CatalogSurface }) => ReactElement;
  const ServiceChips = services.ServiceChips as (props: { categories: { id: string; name: string }[]; selected: string; onSelect: (id: string) => void }) => ReactElement;
  const html = renderToStaticMarkup(createElement(ServiceResults, { surface: fixtureSurface() }));
  const rows = html.match(/<div class="account-row service-row"/g) ?? [];
  assert.ok(rows.length > 0 && rows.length <= CATALOG_RENDER_CAP, `row count ${rows.length}`);
  assert.equal((html.match(/<div class="account-row service-row"/g) ?? []).length, 10, 'initial page shows 10 rows');
  assert.ok(html.includes('Showing 10 of 1,500 services'), html.slice(0, 200));
  assert.match(html, /class="service-count"/);
  assert.ok(html.includes('>Show 10 more</button>'));
  assert.equal(CATALOG_STEP, 10);
  assert.equal(Math.min(CATALOG_STEP + 10, CATALOG_RENDER_CAP), 20, 'Show 10 more increments by exactly 10');
  assert.equal(chipsFromCategories([{ id: 'a', name: 'A' }, { id: 'b', name: 'B' }]).length, 3);
  const many = Array.from({ length: 30 }, (_, index) => ({ id: `cat-${index}`, name: `Category ${index}` }));
  const chipsHtml = renderToStaticMarkup(createElement(ServiceChips, { categories: many, selected: '', onSelect: () => {} }));
  const chipButtons = chipsHtml.match(/<button/g) ?? [];
  assert.ok(chipButtons.length <= 12, `chip count ${chipButtons.length}`);
  assert.ok(chipsHtml.includes('>All</button>'));
  assert.ok(chipsHtml.indexOf('>All</button>') < chipsHtml.indexOf('>Category 0</button>'), 'All is first');
  assert.match(chipsHtml, /aria-pressed="(true|false)"/);

  // 3. Onboarding escape hatch and never-blocked continue.
  const settingsSource = readFileSync(new URL('./Settings.tsx', import.meta.url), 'utf8');
  const footer = settingsSource.slice(settingsSource.indexOf('<footer className="onboarding-footer">'));
  assert.match(footer, /step === 2 && <button className="text-button muted" type="button" disabled=\{busy\} onClick=\{\(\) => void advance\(step \+ 1\)\}>Connect later<\/button>/);
  assert.match(footer, /aria-label=\{`Step \$\{step \+ 1\} of 4`\}/);
  const continueButton = footer.match(/<button className="button primary" type="button" disabled=\{([^}]+)\}/)?.[1] ?? '';
  assert.equal(continueButton, 'busy', 'Continue is disabled only by the footer busy flag');
  assert.match(settingsSource, /Connect your services\./);
  assert.match(settingsSource, /Connecting opens your default browser/);

  // 4. Honest states: skeletons, retryable snapshot errors, empty echoes, fixed-width connecting.
  const loading = renderToStaticMarkup(createElement(ServiceResults, { surface: fixtureSurface({ loading: true, items: [], totalItems: 0 }) }));
  assert.equal((loading.match(/class="skeleton service-skeleton"/g) ?? []).length, 6);
  assert.ok(!loading.includes('account-row'));
  assert.ok(loading.includes('role="status"'));
  const errored = renderToStaticMarkup(createElement(ServiceResults, { surface: fixtureSurface({ error: 'The Composio connector service returned an error.', items: [], totalItems: 0 }) }));
  assert.ok(errored.includes('inline-error') && errored.includes('Try again'));
  const empty = renderToStaticMarkup(createElement(ServiceResults, { surface: fixtureSurface({ items: [], totalItems: 0, query: 'gits' }) }));
  assert.ok(empty.includes('search-empty') && empty.includes('“gits”') && empty.includes('Clear search'));
  const ServiceRow = services.ServiceRow as (props: { service: ComposioService; connecting: boolean; onCancelConnect: () => void; onConnect: () => void }) => ReactElement;
  const connecting = renderToStaticMarkup(createElement(ServiceRow, { service: fixtureService(0), connecting: true, onCancelConnect: () => {}, onConnect: () => {} }));
  assert.match(connecting, /class="button secondary service-connect-button is-connecting"/);
  assert.ok(connecting.includes('Connecting…') && connecting.includes('>Cancel</button>') && connecting.includes('role="status"'));

  // 5. No false connection claims.
  const available = renderToStaticMarkup(createElement(ServiceResults, { surface: fixtureSurface() }));
  assert.doesNotMatch(available, /Connected/);
  assert.ok(!available.includes('CheckCircle'));
  const ConnectedServiceRow = services.ConnectedServiceRow as (props: { item: { id: string; service: string; status: string; statusDetail: string | null; alias: string | null; wordId: string | null; connectedAt: string | null }; busy: boolean; onDisconnect: (id: string) => void }) => ReactElement;
  const connectedHtml = renderToStaticMarkup(createElement(ConnectedServiceRow, {
    item: { id: 'abc123def456', service: 'github', status: 'connected', statusDetail: null, alias: 'work@forma.dev', wordId: 'github_red-castle', connectedAt: '2026-09-12T00:00:00Z' },
    busy: false, onDisconnect: () => {},
  }));
  assert.ok(connectedHtml.includes('>Connected</span>'));
  assert.ok(connectedHtml.includes('work@forma.dev'));
  const attention = renderToStaticMarkup(createElement(ConnectedServiceRow, {
    item: { id: 'abc123def457', service: 'linear', status: 'needs_attention', statusDetail: 'oauth_denied', alias: null, wordId: null, connectedAt: null },
    busy: false, onDisconnect: () => {},
  }));
  assert.ok(attention.includes('Needs attention') && attention.includes('oauth_denied'));

  // 6. Authorization routing stays native; honesty copy present.
  const servicesTsx = readFileSync(new URL('./services.tsx', import.meta.url), 'utf8');
  const controllerSource = readFileSync(new URL('./services-controller.ts', import.meta.url), 'utf8');
  assert.doesNotMatch(servicesTsx, /authorizeUrl|window\.open|browser\.open/);
  assert.doesNotMatch(controllerSource, /authorizeUrl|window\.open|browser\.open/);
  assert.match(controllerSource, /invoke\('composio_start', \{ service \}\)/);
  assert.match(controllerSource, /listen(<ServicePrompt>)?\('composio:connect-prompt'/);
  assert.match(controllerSource, /listen\('composio:changed'/);
  assert.match(servicesTsx, /Each connection is your explicit choice; Forma asks again before anything is sent\./);

  // 7. Chat inertness boundary: the connect card is host-owned, never message content.
  const messageContractsSource = readFileSync(new URL('./message-contracts.ts', import.meta.url), 'utf8');
  assert.doesNotMatch(messageContractsSource, /'connect'/);
  const ServiceConnectCard = services.ServiceConnectCard as (props: {
    service: string; phase: string; reason: string; identity: string | null; error: string | null;
    onConnect: () => void; onCancelConnect: () => void; onDismiss: () => void; onRetry: () => void; onManage: () => void;
  }) => ReactElement;
  const card = (phase: string) => renderToStaticMarkup(createElement(ServiceConnectCard, {
    service: 'notion', phase, reason: 'Forma needs Notion to continue with what you asked.', identity: 'work@forma.dev',
    error: phase === 'failed' ? 'Composio rejected the stored API key.' : null,
    onConnect: () => {}, onCancelConnect: () => {}, onDismiss: () => {}, onRetry: () => {}, onManage: () => {},
  }));
  const idle = card('idle');
  assert.ok(idle.includes('class="connect-card"'));
  assert.ok(!idle.includes('message-card'));
  assert.ok(!idle.includes('message-blocks'));
  assert.match(idle, /<span class="sr-only" role="status">Forma needs Notion\. A connect button is available\.<\/span>/);

  // 8. Accessibility contracts.
  assert.ok(html.includes('aria-label="Service catalog"'));
  const ServiceCatalog = services.ServiceCatalog as (props: { surface: CatalogSurface }) => ReactElement;
  const catalogHtml = renderToStaticMarkup(createElement(ServiceCatalog, { surface: fixtureSurface({ items: [fixtureService(0)], totalItems: 1, categories: many }) }));
  assert.ok(catalogHtml.includes('aria-label="Search services"'));
  assert.ok(catalogHtml.includes('aria-label="Service categories"'));
  assert.ok(idle.includes('aria-label="Connect Notion"'));
  const appSource = readFileSync(new URL('./App.tsx', import.meta.url), 'utf8');
  assert.match(appSource, /aria-current=\{view === 'services' \? 'page' : undefined\}/);
  assert.match(appSource, /className=\{`browser-nav \$\{view === 'services' \? 'selected' : ''\}`\}/);

  // 9. Entry points: view branch, skip link, slash focus, palette action.
  assert.match(appSource, /view === 'services' \? <ServicesView controller=\{services\} onSettings=\{\(\) => setSettingsOpen\(true\)\} \/>/);
  assert.match(appSource, /view === 'services' \? '#services-title'/);
  assert.match(appSource, /view === 'services'\) document\.querySelector<HTMLInputElement>\('\[aria-label="Search services"\]'\)\?\.focus\(\)/);
  assert.match(appSource, /\{ id: 'services', label: 'Services', icon: Plugs, shortcut: '', run: onServices \}/);

  // 10. Connect-card lifecycle honesty.
  assert.ok(idle.includes('>Connect Notion') && idle.includes('>Not now</button>'));
  const connectingCard = card('connecting');
  assert.ok(connectingCard.includes('Connecting…') && connectingCard.includes('>Cancel</button>') && connecting.includes('disabled'));
  const success = card('success');
  assert.ok(success.includes('Connected · Notion · work@forma.dev') && success.includes('Manage in Services'));
  assert.ok(!success.includes('connect-card'), 'success collapses the card');
  const failed = card('failed');
  assert.ok(failed.includes('inline-error') && failed.includes('Try again') && failed.includes('>Not now</button>'));
  const dismissed = card('dismissed');
  assert.ok(dismissed.includes('You chose to connect Notion later.') && dismissed.includes('>Connect now</button>'));

  // Pure prompt governance and status helpers.
  const first = { workspaceId: 'w1', runId: 'r1', service: 'github' };
  assert.equal(mergeServicePrompt([], first).length, 1);
  assert.deepEqual(mergeServicePrompt([first], { ...first, runId: 'r2' }), [{ ...first, runId: 'r2' }], 'dedupe per workspace+service');
  assert.equal(promptsForWorkspace([first], ['w1:github'], 'w1')[0].dismissed, true);
  assert.equal(promptsForWorkspace([first], [], 'w2').length, 0);
  assert.equal(connectCardPhase('github', null, []), 'idle');
  assert.equal(connectCardPhase('github', { id: 'a', service: 'github', connectedAccountId: null, phase: 'pending', expiresAt: '', error: null, detail: null }, []), 'connecting');
  assert.equal(connectCardPhase('github', null, [{ id: 'x', service: 'github', status: 'connected', statusDetail: null, alias: null, wordId: null, connectedAt: null }]), 'success');
  assert.equal(catalogCountLine(10, 1500), 'Showing 10 of 1,500 services');
  // Review round 1 regressions.
  // F5: the TS key rule mirrors the native host rule (ak_ + >=13 = >=16 chars).
  assert.ok(validComposioKey(`ak_${'a'.repeat(13)}`));
  assert.ok(!validComposioKey(`ak_${'a'.repeat(12)}`), 'below the 16-char host minimum');
  assert.ok(!validComposioKey('sk_nope') && !validComposioKey('ak_short'));
  // F7: cancel/disconnect surface failures instead of unhandled rejections.
  assert.ok((controllerSource.match(/catch \(failure\) \{ setConnectError\(message\(failure\)\); \}/g) ?? []).length >= 2,
    'cancel and disconnect report errors');
  // F8: no duplicate initial catalog fetch from the mount effect.
  assert.doesNotMatch(controllerSource, /void loadCatalog\('', ''\);/);
  // F9: no renderer polling interval; snapshots reconcile from native events
  // and keep last known items while an attempt is live.
  assert.doesNotMatch(controllerSource, /setInterval/);
  assert.match(controllerSource, /isAttemptLive\(next\.attempt\)/);
  // F3: dismissal lifts host-side prompt suppression.
  assert.match(controllerSource, /invoke\('composio_prompt_dismiss', \{ workspaceId, service \}\)/);

  // Native lane parity: the Rust host owns every Composio call and secret.
  const native = readFileSync(new URL('../../src-tauri/src/connectors/composio.rs', import.meta.url), 'utf8');
  assert.match(native, /const BASE_URL: &str = "https:\/\/backend\.composio\.dev\/api\/v3";/);
  assert.match(native, /super::opener::spawn\(&link\.redirect_url\)/);
  assert.doesNotMatch(native, /(?:println!|dbg!|log::|tracing::)/);
  const lib = readFileSync(new URL('../../src-tauri/src/lib.rs', import.meta.url), 'utf8');
  for (const command of ['composio_status', 'composio_catalog', 'composio_categories', 'composio_start', 'composio_cancel', 'composio_disconnect', 'composio_key_save', 'composio_key_remove']) {
    assert.ok(lib.includes(`connectors::composio::${command}`), `missing command registration ${command}`);
  }

  return [
    'services css keeps token and spacing discipline with bounded catalog geometry',
    'catalog caps at 30 rows with honest tabular counts and a 12-chip ceiling',
    'onboarding services step keeps Connect later and an unblocked Continue',
    'loading, snapshot-error, empty-search and connecting states stay honest',
    'available rows never claim Connected; connected rows show account identity',
    'all connection routing stays native with consent copy asserted',
    'connect card is host-owned and never part of inert message content',
    'a11y contracts for catalog, chips, sidebar and card hold',
    'services view branch, skip link, slash focus and palette action are wired',
    'connect-card lifecycle idle/connecting/success/failed/dismissed passes static renders',
  ];
}
