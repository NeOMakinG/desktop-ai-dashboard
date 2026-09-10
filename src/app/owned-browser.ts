import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { acceptsBrowserRevision, browserActionFailure, type BrowserEngine, type BrowserService, type OwnedBrowserStatus } from './browser-contracts';

const unavailable: OwnedBrowserStatus = {
  revision: 0, available: false, phase: 'unavailable', engine: 'unavailable', persistent: false,
  availableEngines: [], chromiumUnavailableReason: null,
  profileId: null, url: null, service: null, error: null, automationReady: false,
};

function message(error: unknown): string {
  if (typeof error === 'object' && error && 'message' in error && typeof error.message === 'string') return error.message;
  return 'The browser could not finish that action. Please try again.';
}

export function useOwnedBrowser(native: boolean) {
  const [status, setStatus] = useState<OwnedBrowserStatus>(unavailable);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(false);
  const operation = useRef(false);
  const acceptedRevision = useRef(-1);
  const acceptStatus = useCallback((next: OwnedBrowserStatus) => {
    if (alive.current && acceptsBrowserRevision(acceptedRevision.current, next.revision)) {
      acceptedRevision.current = next.revision;
      setStatus(next);
    }
  }, []);

  const refresh = useCallback(async () => {
    if (!native) return unavailable;
    const next = await invoke<OwnedBrowserStatus>('browser_status');
    acceptStatus(next);
    return next;
  }, [native, acceptStatus]);

  useEffect(() => {
    alive.current = true;
    let release: (() => void) | undefined;
    let disposed = false;
    if (native) {
      void refresh().catch(failure => { if (!disposed) setError(message(failure)); });
      void listen<OwnedBrowserStatus>('owned-browser:state', event => {
        if (!disposed) acceptStatus(event.payload);
      }).then(unlisten => { if (disposed) unlisten(); else release = unlisten; }).catch(() => {
        if (!disposed) setError('Browser status updates are unavailable. Refresh to check the browser state.');
      });
    }
    return () => { alive.current = false; disposed = true; release?.(); };
  }, [native, refresh, acceptStatus]);

  const run = useCallback(async (command: string, args?: Record<string, unknown>) => {
    if (!native) {
      setError('Open the desktop app to use your Forma browser.');
      return false;
    }
    if (operation.current) return false;
    operation.current = true;
    setBusy(true);
    setError(null);
    try {
      const next = await invoke<OwnedBrowserStatus>(command, args);
      acceptStatus(next);
      const failure = browserActionFailure(next) || browserActionFailure(await refresh());
      if (failure) {
        if (alive.current) setError(failure);
        return false;
      }
      return true;
    } catch (failure) {
      if (alive.current) setError(message(failure));
      return false;
    } finally {
      operation.current = false;
      if (alive.current) setBusy(false);
    }
  }, [native, refresh, acceptStatus]);

  return {
    native, status, actionPending: busy, busy: busy || status.phase === 'opening', error: error || status.error,
    clearError: () => setError(null),
    refresh: () => refresh().then(next => { setError(null); return next; }).catch(failure => { setError(message(failure)); return unavailable; }),
    open: (service: BrowserService = 'home', url?: string, engine: BrowserEngine = 'webkit') => run('browser_open', { service, ...(url ? { url } : {}), engine }),
    navigate: (url: string) => run('browser_navigate', { url }),
    back: () => run('browser_back'),
    forward: () => run('browser_forward'),
    reload: () => run('browser_reload'),
    close: () => run('browser_close'),
  };
}

export type OwnedBrowser = ReturnType<typeof useOwnedBrowser>;
