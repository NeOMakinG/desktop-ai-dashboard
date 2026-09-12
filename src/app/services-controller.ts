import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import {
  isAttemptLive,
  mergeServicePrompt,
  promptsForWorkspace,
  type CatalogResult,
  type ComposioService,
  type LiveServicePrompt,
  type ServiceCategory,
  type ServicePrompt,
  type ServicesSnapshot,
} from './services-contracts';

function message(error: unknown): string {
  if (typeof error === 'object' && error && 'message' in error && typeof error.message === 'string') return error.message;
  return 'The service connector could not finish that action.';
}

export interface CatalogState {
  loading: boolean;
  error: string | null;
  items: ComposioService[];
  totalItems: number;
}

export function useServices(native: boolean) {
  const [snapshot, setSnapshot] = useState<ServicesSnapshot | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [catalog, setCatalog] = useState<CatalogState>({ loading: false, error: null, items: [], totalItems: 0 });
  const [categories, setCategories] = useState<ServiceCategory[]>([]);
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState('');
  const [connectError, setConnectError] = useState<string | null>(null);
  const alive = useRef(false);
  const searchEpoch = useRef(0);

  const refreshStatus = useCallback(async () => {
    if (!native) return;
    try {
      const next = await invoke<ServicesSnapshot>('composio_status');
      if (alive.current) { setSnapshot(next); setStatusError(null); }
    } catch (failure) {
      if (alive.current) setStatusError(message(failure));
    }
  }, [native]);

  const loadCatalog = useCallback(async (search: string, activeCategory: string) => {
    if (!native) return;
    const epoch = ++searchEpoch.current;
    setCatalog(current => ({ ...current, loading: true, error: null }));
    try {
      const page = await invoke<CatalogResult>('composio_catalog', {
        search: search || null,
        category: activeCategory || null,
        limit: 100,
      });
      if (!alive.current || epoch !== searchEpoch.current) return;
      setCatalog({ loading: false, error: null, items: page.items, totalItems: page.totalItems });
    } catch (failure) {
      // Loading failures keep the last good rows visible; never fake a catalog.
      if (!alive.current || epoch !== searchEpoch.current) return;
      setCatalog(current => ({ ...current, loading: false, error: message(failure) }));
    }
  }, [native]);

  useEffect(() => {
    alive.current = true;
    let disposed = false;
    let release: (() => void) | undefined;
    if (native) {
      void invoke<ServiceCategory[]>('composio_categories')
        .then(items => { if (!disposed) setCategories(items); })
        .catch(() => { /* Categories are optional; the catalog still loads. */ });
      void listen('composio:changed', () => { if (!disposed) void refreshStatus(); })
        .then(unlisten => { if (disposed) unlisten(); else release = unlisten; })
        .catch(() => { /* Periodic reconciliation below covers dropped events. */ });
      void refreshStatus();
      void loadCatalog('', '');
    }
    return () => { alive.current = false; disposed = true; release?.(); };
  }, [native, refreshStatus, loadCatalog]);

  // Debounced live search; category switches reload immediately.
  useEffect(() => {
    if (!native) return;
    const timer = setTimeout(() => { void loadCatalog(query.trim(), category); }, query ? 300 : 0);
    return () => clearTimeout(timer);
  }, [query, category, native, loadCatalog]);

  const attemptLive = isAttemptLive(snapshot?.attempt ?? null);
  useEffect(() => {
    if (!native || !attemptLive) return;
    const timer = setInterval(() => { void refreshStatus(); }, 2000);
    return () => clearInterval(timer);
  }, [native, attemptLive, refreshStatus]);

  const connect = useCallback(async (service: string) => {
    if (!native) { setConnectError('Open the desktop app to connect services.'); return; }
    setConnectError(null);
    try {
      await invoke('composio_start', { service });
      await refreshStatus();
    } catch (failure) {
      setConnectError(message(failure));
      await refreshStatus();
    }
  }, [native, refreshStatus]);
  const cancelConnect = useCallback(async (attemptId: string) => {
    try { await invoke('composio_cancel', { attemptId }); }
    finally { await refreshStatus(); }
  }, [refreshStatus]);
  const disconnect = useCallback(async (connectedAccountId: string) => {
    try { await invoke('composio_disconnect', { connectedAccountId }); }
    finally { await refreshStatus(); }
  }, [refreshStatus]);
  const saveKey = useCallback(async (apiKey: string) => {
    await invoke('composio_key_save', { apiKey });
    await refreshStatus();
    void loadCatalog(query.trim(), category);
  }, [refreshStatus, loadCatalog, query, category]);
  const removeKey = useCallback(async () => {
    await invoke('composio_key_remove');
    await refreshStatus();
  }, [refreshStatus]);

  return {
    snapshot, statusError, catalog, categories, query, category, connectError,
    setQuery, setCategory, refreshStatus,
    retryCatalog: () => { void loadCatalog(query.trim(), category); },
    connect, cancelConnect, disconnect, saveKey, removeKey,
  };
}
export type ServicesController = ReturnType<typeof useServices>;

export function useServicePrompts(native: boolean, enabled: boolean) {
  const [prompts, setPrompts] = useState<ServicePrompt[]>([]);
  const [dismissed, setDismissed] = useState<string[]>([]);
  useEffect(() => {
    if (!native || !enabled) return;
    let disposed = false;
    let release: (() => void) | undefined;
    // Only the native host emits this structured event; message text can never
    // create or update a connect card.
    void listen<ServicePrompt>('composio:connect-prompt', event => {
      if (!disposed) setPrompts(current => mergeServicePrompt(current, event.payload));
    }).then(unlisten => { if (disposed) unlisten(); else release = unlisten; }).catch(() => {});
    return () => { disposed = true; release?.(); };
  }, [native, enabled]);
  const dismiss = useCallback((workspaceId: string, service: string) => {
    setDismissed(current => [...current, `${workspaceId}:${service}`]);
  }, []);
  const restore = useCallback((workspaceId: string, service: string) => {
    setDismissed(current => current.filter(key => key !== `${workspaceId}:${service}`));
  }, []);
  const forWorkspace = useCallback((workspaceId: string): LiveServicePrompt[] =>
    promptsForWorkspace(prompts, dismissed, workspaceId), [prompts, dismissed]);
  return { forWorkspace, dismiss, restore };
}
