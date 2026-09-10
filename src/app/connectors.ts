import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { ConnectorCapabilities, ConnectorStatus } from './browser-contracts';
import './connectors.css';

export const GOOGLE_DEFAULT_SCOPES = [
  'https://www.googleapis.com/auth/gmail.metadata',
  'https://www.googleapis.com/auth/calendar.readonly',
] as const;

const CAPABILITIES_UNAVAILABLE: ConnectorCapabilities = {
  googleAvailable: false,
  disabledReason: 'Google OAuth client not configured. A supported desktop client and consent setup are required.',
  defaultScopes: [...GOOGLE_DEFAULT_SCOPES],
};

type Attempt = {
  id: string;
  phase: 'pending' | 'exchanging' | 'connected' | 'cancelled' | 'failed';
  expiresAt: string;
  error: { code: string; message: string } | null;
};
type Snapshot = { revision: number; attempt: Attempt | null; items: ConnectorStatus[]; cleanupIds: string[] };
type StartResponse = { attemptId: string };

function message(error: unknown): string {
  if (typeof error === 'object' && error && 'message' in error && typeof error.message === 'string') return error.message;
  return 'The connector could not finish that action.';
}

export function useConnectors(native: boolean) {
  const [items, setItems] = useState<ConnectorStatus[]>([]);
  const [cleanupIds, setCleanupIds] = useState<string[]>([]);
  const [capabilities, setCapabilities] = useState<ConnectorCapabilities>(CAPABILITIES_UNAVAILABLE);
  const [attempt, setAttempt] = useState<Attempt | null>(null);
  const [operating, setOperating] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const alive = useRef(false);
  const session = useRef(0);
  const operation = useRef(false);
  const cancellation = useRef(false);
  const current = useRef<Snapshot | null>(null);
  const request = useRef(0);
  const pending = attempt?.phase === 'pending' || attempt?.phase === 'exchanging';

  const refresh = useCallback(async () => {
    if (!native) return;
    const epoch = session.current;
    const sequence = ++request.current;
    try {
      const next = await invoke<Snapshot>('connectors_status');
      if (!alive.current || epoch !== session.current || next.revision < (current.current?.revision ?? -1)) return;
      const previous = current.current?.attempt;
      current.current = next;
      setItems(next.items); setAttempt(next.attempt); setCleanupIds(next.cleanupIds);
      if (next.attempt?.id !== previous?.id || next.attempt?.phase !== previous?.phase) {
        if (next.attempt?.error) setError(next.attempt.error.message);
        if (next.attempt?.phase === 'cancelled') setNotice('Sign-in cancelled.');
        if (next.attempt?.phase === 'connected') setNotice('Google permissions saved. Mail and calendar reading is not available yet.');
        if (next.attempt?.phase === 'pending') { setNotice(null); setError(null); }
      }
    } catch (failure) {
      if (alive.current && epoch === session.current && sequence === request.current) setError(message(failure));
    }
  }, [native]);

  useEffect(() => {
    alive.current = true;
    let release: (() => void) | undefined;
    let disposed = false;
    let timer: ReturnType<typeof setInterval> | undefined;
    if (native) {
      void invoke<ConnectorCapabilities>('connectors_capabilities')
        .then(caps => { if (!disposed) setCapabilities(caps); })
        .catch(() => { /* Keep the unavailable default; never infer readiness. */ });
      // Subscribe before first snapshot; periodic reconciliation covers dropped events
      // and a Settings remount during a pending attempt.
      void listen<void>('connectors:changed', () => { if (!disposed) void refresh(); })
        .then(unlisten => {
          if (disposed) unlisten();
          else { release = unlisten; void refresh(); }
        }).catch(() => { if (!disposed) void refresh(); });
      timer = setInterval(() => { void refresh(); }, 1000);
    }
    return () => {
      alive.current = false; disposed = true; session.current += 1;
      release?.(); if (timer) clearInterval(timer);
    };
  }, [native, refresh]);

  const startGoogle = useCallback(async (scopes: readonly string[] = GOOGLE_DEFAULT_SCOPES): Promise<StartResponse | null> => {
    if (!native) { setError('Open the desktop app to sign in.'); return null; }
    if (!capabilities.googleAvailable) { setError(capabilities.disabledReason || CAPABILITIES_UNAVAILABLE.disabledReason); return null; }
    if (operation.current || cancellation.current || ['pending', 'exchanging'].includes(current.current?.attempt?.phase ?? '')) return null;
    const epoch = session.current;
    operation.current = true; setOperating(true); setError(null); setNotice(null);
    try {
      const response = await invoke<StartResponse>('connectors_start_google', { scopes: [...scopes] });
      await refresh();
      if (!alive.current || epoch !== session.current) return null;
      // Native opens the system browser; the renderer receives only attempt identity.
      return response;
    } catch (failure) {
      if (alive.current && epoch === session.current) setError(message(failure));
      await refresh();
      return null;
    } finally {
      operation.current = false;
      if (alive.current && epoch === session.current) setOperating(false);
    }
  }, [native, capabilities.googleAvailable, capabilities.disabledReason, refresh]);

  const cancel = useCallback(async (attemptId: string = current.current?.attempt?.id ?? '') => {
    if (!native || !attemptId || cancellation.current) return;
    const epoch = session.current;
    cancellation.current = true; setCancelling(true); setError(null);
    try {
      await invoke('connectors_cancel', { attemptId });
    } catch (failure) {
      if (alive.current && epoch === session.current) setError(message(failure));
    } finally {
      await refresh();
      cancellation.current = false;
      if (alive.current && epoch === session.current) setCancelling(false);
    }
  }, [native, refresh]);

  const disconnect = useCallback(async (id: string) => {
    if (!native || operation.current) return;
    const epoch = session.current;
    operation.current = true; setOperating(true); setError(null); setNotice(null);
    try {
      await invoke('connectors_disconnect', { id });
      if (alive.current && epoch === session.current) setNotice('Disconnected from Forma. This does not revoke access at Google.');
    } catch (failure) {
      if (alive.current && epoch === session.current) setError(message(failure));
    } finally {
      await refresh();
      operation.current = false;
      if (alive.current && epoch === session.current) setOperating(false);
    }
  }, [native, refresh]);

  const refreshTokens = useCallback(async (id: string) => {
    if (!native || operation.current) return null;
    const epoch = session.current;
    operation.current = true; setOperating(true); setError(null);
    try {
      const status = await invoke<ConnectorStatus>('connectors_refresh', { id });
      if (!alive.current || epoch !== session.current) return null;
      return status;
    } catch (failure) {
      if (alive.current && epoch === session.current) setError(message(failure));
      return null;
    } finally {
      await refresh();
      operation.current = false;
      if (alive.current && epoch === session.current) setOperating(false);
    }
  }, [native, refresh]);

  return {
    items, cleanupIds, capabilities, attempt, pending, cancelling,
    busy: operating || pending || cancelling,
    error, notice,
    clearError: () => setError(null),
    refresh, startGoogle, cancel, refreshTokens, disconnect,
  };
}

export type ConnectorsController = ReturnType<typeof useConnectors>;
