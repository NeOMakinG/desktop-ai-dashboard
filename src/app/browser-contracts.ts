export type BrowserService = 'home' | 'gmail' | 'google_calendar' | 'custom';
export type BrowserPhase = 'closed' | 'opening' | 'open' | 'error' | 'unavailable';
export type BrowserEngine = 'webkit' | 'chromium' | 'scrapling';
export interface AvailableBrowserEngine { id: BrowserEngine; label: string }

/** Lifecycle of the REAL Chrome-for-Testing browser (install → run). */
export type RealChromiumPhase =
  | 'idle' | 'downloading' | 'extracting' | 'verifying' | 'launching' | 'running' | 'exited' | 'error';

/** Which real-Chrome binary launches: the user's own installed Google Chrome
 * (preferred — real browser identity) or the bundled Chrome for Testing. */
export type RealChromiumBinary = 'system_chrome' | 'chrome_for_testing' | null;

export interface RealChromiumStatus {
  supported: boolean;
  installed: boolean;
  binary: RealChromiumBinary;
  phase: RealChromiumPhase;
  progressPercent: number | null;
  pid: number | null;
  cdpReady: boolean;
  error: string | null;
  version: string;
}

/** Honest label for the binary actually in use. */
export function realChromiumBinaryLabel(binary: RealChromiumBinary): string | null {
  if (binary === 'system_chrome') return 'Using your installed Google Chrome (with a separate Forma profile).';
  if (binary === 'chrome_for_testing') return 'Using the bundled Chrome for Testing build.';
  return null;
}

/** Honest one-line label for the real-browser status card. */
export function realChromiumStatusLine(real: RealChromiumStatus): string {
  switch (real.phase) {
    case 'downloading': return `Downloading Chrome ${real.version}… ${real.progressPercent ?? 0}%`;
    case 'extracting': return 'Extracting Chrome…';
    case 'verifying': return 'Verifying the Chrome install…';
    case 'launching': return 'Starting Chrome…';
    case 'running': return `Chrome is running${real.pid ? ` (pid ${real.pid})` : ''}${real.cdpReady ? ' · assistant link ready' : ''}`;
    case 'exited': return 'Chrome was closed. Open it again any time — your sign-ins are kept.';
    case 'error': return real.error || 'Chrome could not be prepared.';
    default:
      if (real.binary === 'system_chrome') return 'Your installed Google Chrome is ready to open inside Forma.';
      return real.installed
        ? 'Chrome is installed and ready to open.'
        : `Chrome ${real.version} downloads on first use (about 190 MB).`;
  }
}

export function browserActionFailure(status: OwnedBrowserStatus): string | null {
  if (status.phase === 'error' || status.phase === 'unavailable' || !status.available) {
    return status.error || 'The owned browser is not available on this build.';
  }
  return null;
}

export function acceptsBrowserRevision(current: number, incoming: number): boolean {
  return Number.isSafeInteger(incoming) && incoming >= 0 && incoming >= current;
}

export interface OwnedBrowserStatus {
  revision: number;
  available: boolean;
  phase: BrowserPhase;
  engine: BrowserEngine | 'unavailable';
  availableEngines: AvailableBrowserEngine[];
  chromiumUnavailableReason: string | null;
  realChromium: RealChromiumStatus;
  persistent: boolean;
  profileId: string | null;
  url: string | null;
  service: BrowserService | null;
  error: string | null;
  automationReady: boolean;
  /** False only for Scrapling snapshot sessions: fetched pages render read-only. */
  interactive: boolean;
  /** True while the Scrapling engine is fetching the next snapshot. */
  navigating: boolean;
}

/** The engine `browser_open` uses when the caller omits one: the first host
 * advertised engine (Scrapling when its sealed resources verify, else WebKit). */
export function defaultBrowserEngine(engines: AvailableBrowserEngine[]): BrowserEngine | 'unavailable' {
  const id = engines[0]?.id;
  return id ?? 'unavailable';
}

export type ConnectorProvider = 'google';

export interface ConnectorStatus {
  id: string;
  provider: ConnectorProvider;
  scopes: string[];
  displayName: string | null;
  connectedAt: string;
  expiresAt: string;
}

export interface ConnectorCapabilities {
  googleAvailable: boolean;
  disabledReason: string | null;
  defaultScopes: string[];
}

// ---------------------------------------------------------------------------
// Embedded Chrome surface: the real browser rendered inside the Forma window
// via CDP screencast. Pure geometry/input helpers live here so the regression
// suite can exercise them without a DOM.
// ---------------------------------------------------------------------------

export interface EmbeddedTab {
  id: string;
  url: string;
  title: string;
  loading: boolean;
  active: boolean;
}

export interface EmbeddedState {
  running: boolean;
  /** False during a transient CDP reconnect (browser still alive). */
  connected: boolean;
  poppedOut: boolean;
  tabs: EmbeddedTab[];
  activeId: string | null;
}

export const emptyEmbeddedState: EmbeddedState = { running: false, connected: true, poppedOut: false, tabs: [], activeId: null };

export interface EmbeddedFrameMeta {
  /** CSS-pixel size of Chrome's viewport when the frame was captured. */
  deviceWidth: number;
  deviceHeight: number;
}

export interface FitRect { x: number; y: number; width: number; height: number }

/** Contain-fit a frame of `frameW x frameH` into a canvas of `boxW x boxH`
 * (letterboxed, centered, aspect preserved). All units are the caller's. */
export function embeddedContainRect(boxW: number, boxH: number, frameW: number, frameH: number): FitRect {
  if (boxW <= 0 || boxH <= 0 || frameW <= 0 || frameH <= 0) return { x: 0, y: 0, width: 0, height: 0 };
  const scale = Math.min(boxW / frameW, boxH / frameH);
  const width = frameW * scale;
  const height = frameH * scale;
  return { x: (boxW - width) / 2, y: (boxH - height) / 2, width, height };
}

/** Map a pointer position (in canvas CSS pixels) to page CSS coordinates.
 * Returns null in the letterbox area — those events are never forwarded. */
export function embeddedPointToPage(px: number, py: number, rect: FitRect, meta: EmbeddedFrameMeta): { x: number; y: number } | null {
  if (rect.width <= 0 || rect.height <= 0) return null;
  if (px < rect.x || py < rect.y || px > rect.x + rect.width || py > rect.y + rect.height) return null;
  return {
    x: ((px - rect.x) / rect.width) * meta.deviceWidth,
    y: ((py - rect.y) / rect.height) * meta.deviceHeight,
  };
}

/** CDP input modifier bitmask: Alt=1, Ctrl=2, Meta/Cmd=4, Shift=8. */
export function cdpModifiers(event: { altKey: boolean; ctrlKey: boolean; metaKey: boolean; shiftKey: boolean }): number {
  return (event.altKey ? 1 : 0) | (event.ctrlKey ? 2 : 0) | (event.metaKey ? 4 : 0) | (event.shiftKey ? 8 : 0);
}

export function cdpMouseButton(button: number): 'left' | 'middle' | 'right' | null {
  return button === 0 ? 'left' : button === 1 ? 'middle' : button === 2 ? 'right' : null;
}

/** App-safe tab shortcuts, active only while the embedded canvas is focused:
 * Cmd/Ctrl+T new tab, Cmd/Ctrl+W close tab. Everything else forwards. */
export function embeddedShortcut(event: { key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean }): 'new_tab' | 'close_tab' | 'paste' | null {
  const command = event.metaKey || event.ctrlKey;
  if (!command || event.altKey || event.shiftKey) return null;
  const key = event.key.toLowerCase();
  if (key === 't') return 'new_tab';
  if (key === 'w') return 'close_tab';
  if (key === 'v') return 'paste';
  return null;
}

/** Normalize what the user typed in the address bar into a navigable URL.
 * Bare domains get https:// — plain http is rejected by the host gate. */
export function normalizeAddress(raw: string): string | null {
  const value = raw.trim();
  if (!value || /\s/.test(value)) return null;
  if (/^https:\/\//i.test(value)) return value;
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(value)) return null; // other schemes: refuse, never rewrite
  if (!value.includes('.')) return null;
  return `https://${value}`;
}

export function isSecureUrl(url: string | null | undefined): boolean {
  return !!url && url.startsWith('https://');
}

/** Compact tab title: page title, else the URL's host, else an honest blank. */
export function embeddedTabLabel(tab: Pick<EmbeddedTab, 'title' | 'url'>): string {
  if (tab.title.trim()) return tab.title.trim();
  try {
    const host = new URL(tab.url).host;
    if (host) return host;
  } catch { /* not a parseable URL */ }
  return tab.url || 'New tab';
}
