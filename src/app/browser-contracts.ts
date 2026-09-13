export type BrowserService = 'home' | 'gmail' | 'google_calendar' | 'custom';
export type BrowserPhase = 'closed' | 'opening' | 'open' | 'error' | 'unavailable';
export type BrowserEngine = 'webkit' | 'chromium' | 'scrapling';
export interface AvailableBrowserEngine { id: BrowserEngine; label: string }

/** Lifecycle of the REAL Chrome-for-Testing browser (install → run). */
export type RealChromiumPhase =
  | 'idle' | 'downloading' | 'extracting' | 'verifying' | 'launching' | 'running' | 'exited' | 'error';

export interface RealChromiumStatus {
  supported: boolean;
  installed: boolean;
  phase: RealChromiumPhase;
  progressPercent: number | null;
  pid: number | null;
  cdpReady: boolean;
  error: string | null;
  version: string;
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
    default: return real.installed
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
