import { useEffect, useState } from 'react';
import { InlineError } from './components';
import { ManagedHermesStatus } from './RuntimeActivity';
import type { AppStore } from './store';
import { CONNECTOR_EGRESS_DISCLOSURE, CONNECTOR_READS_UNAVAILABLE, connectorReadCapabilities, listConnectorGrants, revokeConnectorGrant, type ConnectorGrant } from './connector-grants';
import { friendlyError } from './bridge';

export function RuntimeSettings({ store }: { store: AppStore }) {
  return <section className="settings-section runtime-settings"><h3>Hermes</h3>
    <p className="field-note">Forma manages Hermes automatically. Choose your model and provider above; there is no separate runtime setup.</p>
    <ManagedHermesStatus state={store.getSnapshot()} store={store} />
    <p className="field-note">Custom code unavailable — isolated execution is not verified. Live Google reads remain blocked. Account connections and grants are separate.</p>
  </section>;
}

export function AccountGrantStatus({ native, workspaceId, runtimeOrigin, modelOrigin }: { native: boolean; workspaceId: string | null; runtimeOrigin?: string; modelOrigin?: string }) {
  const [capabilities, setCapabilities] = useState(CONNECTOR_READS_UNAVAILABLE); const [grants, setGrants] = useState<ConnectorGrant[]>([]); const [error, setError] = useState(''); const [busy, setBusy] = useState(false);
  useEffect(() => { let alive = true; if (native) void Promise.all([connectorReadCapabilities(), listConnectorGrants()]).then(([caps, items]) => { if (alive) { setCapabilities(caps); setGrants(items); } }).catch(failure => { if (alive) setError(friendlyError(failure)); }); return () => { alive = false; }; }, [native, workspaceId]);
  const revoke = async (id: string) => { setBusy(true); setError(''); try { await revokeConnectorGrant(id); setGrants(items => items.filter(item => item.grantId !== id)); } catch (failure) { setError(friendlyError(failure)); } finally { setBusy(false); } };
  return <section className="runtime-grants"><h3>Workspace account access</h3><p className="field-note">No account data is selected for this interface/schedule slice. Connecting an account is not permission for an agent to read it.</p>
    <dl className="runtime-facts"><dt>Sent to Hermes runtime</dt><dd>{runtimeOrigin || 'Not available'}</dd><dt>Sent to model destination</dt><dd>{modelOrigin || 'Unknown — account egress blocked'}</dd><dt>Source data</dt><dd>None. Conversation content still reaches the selected runtime/model when you send.</dd></dl>
    <p className="field-note">{CONNECTOR_EGRESS_DISCLOSURE} External runtime/model retention is unknown unless your operator has independently verified it.</p>
    <ul className="runtime-reasons">{capabilities.reasons.map(reason => <li key={reason}>{reason}</li>)}</ul>
    <button type="button" className="button secondary" disabled title="Live Google compliance and retention gates are incomplete">Grant live Google data — unavailable</button>
    {grants.filter(grant => grant.workspaceId === workspaceId).map(grant => <div className="account-row" key={grant.grantId}><span>Grant {grant.connectionId}<small className="row-detail">{grant.operations.join(', ')} · expires {grant.expiresAt} · {grant.deviceId}</small><small className="row-detail">Runtime {grant.egress.runtimeOrigin} · Model {grant.egress.modelOrigin}</small></span><button type="button" className="text-button" disabled={busy} onClick={() => void revoke(grant.grantId)}>Revoke</button></div>)}
    {error && <InlineError>{error}</InlineError>}
  </section>;
}
