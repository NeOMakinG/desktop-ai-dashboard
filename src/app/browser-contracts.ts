export type BrowserService = 'home' | 'gmail' | 'google_calendar' | 'custom';
export type BrowserPhase = 'closed' | 'opening' | 'open' | 'error' | 'unavailable';
export type BrowserEngine = 'webkit' | 'chromium';
export interface AvailableBrowserEngine { id: BrowserEngine; label: string }

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
  persistent: boolean;
  profileId: string | null;
  url: string | null;
  service: BrowserService | null;
  error: string | null;
  automationReady: boolean;
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
