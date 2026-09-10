import { invoke, isTauri } from '@tauri-apps/api/core';
import { useCallback, useEffect, useRef, useState } from 'react';
import { AppError, friendlyError } from './bridge.ts';
import type {
  RuntimeDeleted, RuntimeInterface, RuntimeProgress, RuntimeProposal,
  RuntimeProposalInput, RuntimeRun, RuntimeSchedule, RuntimeScheduleConsent, RuntimeScheduleInput,
  RuntimeStatus, RuntimeWorkspace, RuntimeInterfaceSummary, RuntimeProposalSummary, RuntimeScheduleSummary, RuntimeInterfaceRevisionSummary,
} from './runtime-contracts';
export type * from './runtime-contracts';

function native<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) return Promise.reject(new AppError('browser_unsupported', 'The dedicated Hermes runtime requires the desktop host.'));
  return invoke<T>(command, args);
}
// Only host-owned UI imports this API. Generated interfaces never receive a bridge.
function resource<T>(operation: string, fields: Record<string, unknown> = {}): Promise<T> {
  return native<T>('runtime_resource', { input: { operation, ...fields } });
}
export const runtimeBridge = {
  status: () => native<RuntimeStatus>('runtime_status'),
  retry: () => native<RuntimeStatus>('runtime_retry'),
  check: () => native<RuntimeStatus>('runtime_check'),
  workspace: (workspaceId: string) => native<RuntimeWorkspace>('runtime_workspace', { workspaceId }),
  selectModel: (workspaceId: string, model: string) => native<RuntimeWorkspace>('runtime_select_model', { workspaceId, model }),
  progress: (workspaceId: string, requestId: string) => native<RuntimeProgress>('runtime_progress', { workspaceId, requestId }),
  listInterfaces: (workspaceId: string) => resource<RuntimeInterfaceSummary[]>('listInterfaces', { workspaceId }),
  getInterface: (id: string) => resource<RuntimeInterface>('getInterface', { id }),
  listProposals: (workspaceId: string) => resource<RuntimeProposalSummary[]>('listProposals', { workspaceId }),
  getProposal: (id: string) => resource<RuntimeProposal>('getProposal', { id }),
  getSchedule: (id: string) => resource<RuntimeSchedule>('getSchedule', { id }),
  proposeInterface: (input: RuntimeProposalInput) => resource<RuntimeProposal>('proposeInterface', { input }),
  publishInterface: (proposalId: string, expectedRevision: number) => resource<RuntimeInterface>('publishInterface', { proposalId, expectedRevision }),
  renameInterface: (id: string, expectedRevision: number, title: string) => resource<RuntimeInterface>('renameInterface', { id, expectedRevision, title }),
  interfaceRevisions: (id: string) => resource<RuntimeInterfaceRevisionSummary[]>('interfaceRevisions', { id }),
  rollbackInterface: (id: string, expectedRevision: number, targetRevision: number) => resource<RuntimeInterface>('rollbackInterface', { id, expectedRevision, targetRevision }),
  deleteInterface: (id: string, expectedRevision: number) => resource<RuntimeDeleted>('deleteInterface', { id, expectedRevision }),
  listSchedules: (workspaceId: string) => resource<RuntimeScheduleSummary[]>('listSchedules', { workspaceId }),
  createSchedule: (input: RuntimeScheduleInput) => resource<RuntimeSchedule>('createSchedule', { input }),
  enableSchedule: (id: string, expectedVersion: number, consent: RuntimeScheduleConsent) => resource<RuntimeSchedule>('enableSchedule', { id, expectedVersion, consent }),
  pauseSchedule: (id: string, expectedVersion: number) => resource<RuntimeSchedule>('pauseSchedule', { id, expectedVersion }),
  deleteSchedule: (id: string, expectedVersion: number) => resource<RuntimeDeleted>('deleteSchedule', { id, expectedVersion }),
  listRuns: (workspaceId: string) => resource<RuntimeRun[]>('listRuns', { workspaceId }),
  runHistory: (workspaceId: string) => resource<RuntimeRun[]>('listRuns', { workspaceId }),
  activity: (workspaceId: string) => resource<RuntimeRun[]>('listRuns', { workspaceId }),
};

/** A failed native check may have invalidated the binding before rejecting. Never retain its old UI approval state. */
export async function observeRuntimeAction(
  action: () => Promise<RuntimeStatus>, readStatus: () => Promise<RuntimeStatus>, publish: (status: RuntimeStatus | null) => void,
): Promise<RuntimeStatus> {
  publish(null);
  try { const next = await action(); publish(next); return next; }
  catch (failure) {
    try { publish(await readStatus()); } catch { publish(null); }
    throw failure;
  }
}

/** Configuration fetch is read-only; mounting never checks a server, enables a route or makes a model call. */
export function useRuntime() {
  const [status, setStatus] = useState<RuntimeStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const revision = useRef(0);
  const run = useCallback(async (action: () => Promise<RuntimeStatus>) => {
    const current = ++revision.current;
    setLoading(true); setError(null);
    try { return await observeRuntimeAction(action, runtimeBridge.status, next => { if (revision.current === current) setStatus(next); }); }
    catch (failure) { if (revision.current === current) setError(friendlyError(failure)); throw failure; }
    finally { if (revision.current === current) setLoading(false); }
  }, []);
  const refresh = useCallback(() => run(runtimeBridge.status), [run]);
  const retry = useCallback(() => run(runtimeBridge.retry), [run]);
  const check = useCallback(() => run(runtimeBridge.check), [run]);
  useEffect(() => { const current = ++revision.current; if (isTauri()) void runtimeBridge.status().then(value => { if (revision.current === current) setStatus(value); }).catch(failure => { if (revision.current === current) setError(friendlyError(failure)); }); return () => { revision.current += 1; }; }, []);
  return { status, loading, error, refresh, retry, check };
}
