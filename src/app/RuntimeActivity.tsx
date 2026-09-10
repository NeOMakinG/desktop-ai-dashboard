import { useState } from 'react';
import type { RuntimeEvent, RuntimeProgress, RuntimeStatus } from './runtime-contracts';
import type { AppSnapshot, AppStore } from './store';

export function toolActivity(events: RuntimeEvent[]) {
  const rows = new Map<string, { requestId: string; toolName: string; deviceId?: string; outcome: string; seq: number; at: string; runId: string }>();
  const seen = new Set<string>();
  for (const event of [...events].sort((a, b) => a.seq - b.seq)) {
    const identity = `${event.runId}:${event.seq}`; if (seen.has(identity)) continue; seen.add(identity);
    if (event.type !== 'tool.requested' && event.type !== 'tool.result') continue;
    const key = `${event.runId}:${event.payload.requestId}`; const previous = rows.get(key);
    if (event.type === 'tool.requested') rows.set(key, { requestId: event.payload.requestId, toolName: event.payload.toolName, deviceId: event.payload.deviceId, outcome: previous?.outcome ?? 'Requested', seq: event.seq, at: event.at, runId: event.runId });
    else rows.set(key, { requestId: event.payload.requestId, toolName: previous?.toolName ?? 'Tool operation (earlier event not in this page)', deviceId: previous?.deviceId, outcome: event.payload.outcome, seq: event.seq, at: event.at, runId: event.runId });
  }
  return [...rows.values()];
}
export function RuntimeActivity({ progress, workspaceId, onInterfaces }: { progress: RuntimeProgress | null; workspaceId: string | null; onInterfaces: (id?: string) => void }) {
  if (!progress || progress.workspaceId !== workspaceId) return null;
  const rows = toolActivity(progress.events);
  const references = progress.events.filter(event => event.type === 'interface.updated' || event.type === 'interface.proposed');
  return <section className="runtime-activity" aria-label="Hermes run activity"><h3>Hermes run · {progress.state === 'uncertain' ? 'Unknown after disconnect — reconcile before retrying' : progress.state === 'cancelling' ? 'Cancel requested — not yet confirmed' : progress.state === 'cancelled' ? 'Canceled — confirmed by executor' : progress.state}</h3>
    <p className="field-note">{progress.run ? `Run ${progress.run.id} · accepted model ${progress.run.modelId}` : 'Run admission has not been acknowledged.'} Tool success does not imply the run or interface publication succeeded.</p>
    <div className="runtime-event-list">{rows.map(row => <details key={`${row.runId}:${row.requestId}`}><summary>{row.outcome} · {row.toolName} · {row.deviceId || 'Runtime executor'}</summary><p>Run {row.runId} · request {row.requestId} · sequence {row.seq} · {row.at}</p><p>Arguments and raw account results are withheld by the native host. Only sanitized structured outcomes are displayed.</p></details>)}</div>
    {references.map(event => <button key={`${event.runId}:${event.seq}`} type="button" className="text-button" onClick={() => onInterfaces(event.type === 'interface.updated' ? event.payload.interfaceId : undefined)}>{event.type === 'interface.updated' ? `Open interface · saved r${event.payload.revision}` : 'Review interface proposal · not published'}</button>)}
    {progress.events.filter(event => event.type === 'error').map(event => <p key={`${event.runId}:${event.seq}`} className="field-note">{event.payload.code}: {event.payload.message}</p>)}
    {!rows.length && <p className="field-note">No structured tool activity returned for this run. Assistant prose cannot create tool events.</p>}
  </section>;
}

export function hermesCanAcceptMessage(native: boolean, status: RuntimeStatus | null, providerVerified: boolean, workspaceModel: string): boolean {
  if (!native || !status?.verified || !workspaceModel) return false;
  return status.state === 'ready' ? !!status.capabilities?.runtime.ready : status.state === 'needsModel' && providerVerified;
}

export function managedHermesLabel(native: boolean, status: RuntimeStatus | null): string {
  if (!native) return 'Hermes error · desktop app required';
  if (!status || status.state === 'starting') return 'Hermes starting';
  if (status.state === 'needsModel') return 'Hermes needs a model';
  if (status.state === 'ready' && status.verified && status.capabilities?.runtime.ready) return 'Hermes ready';
  return 'Hermes error';
}

export function ManagedHermesStatus({ state, store, onSettings }: { state: AppSnapshot; store: AppStore; onSettings?: () => void }) {
  const [retrying, setRetrying] = useState(false);
  const [retryError, setRetryError] = useState(false);
  const status = state.runtime;
  const savedModelPending = status?.state === 'needsModel' && state.settings?.provider.verified && state.runtimeWorkspace?.workspaceId === state.activeId && !!state.runtimeWorkspace?.modelId;
  const locked = retrying || state.anyReplyPending || state.navigating || state.selectingModel || state.providerBusy || state.closing;
  const retry = async () => {
    if (locked) return;
    setRetrying(true); setRetryError(false);
    try { if (!await store.retryRuntime()) setRetryError(true); }
    catch { setRetryError(true); }
    finally { setRetrying(false); }
  };
  return <div className="hermes-status"><div className="hermes-status-line"><span role="status">{retrying ? 'Hermes starting' : managedHermesLabel(store.bridge.native, status)}</span>
    {store.bridge.native && status?.state === 'error' && <button type="button" className="text-button" disabled={locked} onClick={() => void retry()}>{retrying ? 'Retrying…' : 'Retry'}</button>}
    {store.bridge.native && status?.state === 'needsModel' && onSettings && <button type="button" className="text-button" disabled={locked} onClick={onSettings}>Model settings</button>}
  </div>{status?.message && <p className="field-note">{status.message}</p>}{savedModelPending && <p className="field-note">Your saved model will connect when you send. No additional setup is needed.</p>}{retryError && <p className="field-note" role="alert">Hermes could not restart. Your saved work is unchanged. Retry when available.</p>}</div>;
}
