import { useCallback, useEffect, useRef, useState } from 'react';
import { ArrowLeft, Plus, ArrowClockwise } from '@phosphor-icons/react';
import { InlineError, Modal } from './components';
import { runtimeBridge, type RuntimeInterface, type RuntimeInterfaceRevisionSummary, type RuntimeProposal, type RuntimeStatus, type RuntimeInterfaceSummary, type RuntimeProposalSummary } from './runtime';
import { libraryError, LibraryValidationError } from './library-errors';
import { emptyInterfaceSpec, validateInterfaceSpec } from './interface-contracts';
import { globalShortcutBlocked, LibraryRequestFence, nextLibraryRow } from './library-interactions';
import { InterfaceRenderer } from './InterfaceRenderer';

export function listKeyboard(event: React.KeyboardEvent<HTMLElement>) {
  const target = event.target as HTMLElement;
  if (event.defaultPrevented || globalShortcutBlocked(event.nativeEvent) || event.metaKey || event.ctrlKey || event.altKey || target.closest('dialog')) return;
  if (!['j', 'k', 'ArrowDown', 'ArrowUp'].includes(event.key)) return;
  const rows = [...event.currentTarget.querySelectorAll<HTMLButtonElement>('[data-library-row]')];
  const current = rows.indexOf(target.closest<HTMLButtonElement>('[data-library-row]')!);
  const next = nextLibraryRow(event.key, current, rows.length, false);
  if (next !== null) { event.preventDefault(); rows[next]?.focus(); }
}
export function runtimeUnavailable(native: boolean, status: RuntimeStatus | null): string | null {
  if (!native) return 'Native runtime unavailable in browser evaluation. Your local chats remain usable; this is not a connected library.';
  if (!status || status.state === 'starting') return 'Hermes is starting automatically. Your saved library has not been changed.';
  if (!status.verified || !status.capabilities) return status.state === 'needsModel' ? 'Hermes needs a model. Choose your model and provider in Settings.' : 'Hermes is unavailable. Check its managed status in Settings and retry.';
  // Native verification covers the authenticated binding, not worker readiness.
  return null;
}
export function runtimeExecutionUnavailable(native: boolean, status: RuntimeStatus | null): string | null {
  return runtimeUnavailable(native, status) || (!status?.capabilities?.runtime.ready ? status?.capabilities?.runtime.reason || 'Hermes executor has not reported readiness.' : status.state !== 'ready' ? 'Hermes has not reported managed readiness.' : !status.capabilities.modelOrigin ? 'The runtime has not verified its model destination.' : null);
}
export function InterfacesView({ active, native, status, workspaceId, focusId, onRevise, onSettings }: {
  active: boolean; native: boolean; status: RuntimeStatus | null; workspaceId: string | null; focusId?: string;
  onRevise: (prompt: string) => void; onSettings: () => void;
}) {
  const [items, setItems] = useState<RuntimeInterfaceSummary[]>([]); const [proposals, setProposals] = useState<RuntimeProposalSummary[]>([]);
  const [selected, setSelected] = useState<RuntimeInterface | null>(null); const [preview, setPreview] = useState<RuntimeProposal | null>(null);
  const [query, setQuery] = useState(''); const [loading, setLoading] = useState(false); const [busy, setBusy] = useState(false); const [error, setError] = useState(''); const [notice, setNotice] = useState('');
  const [dialog, setDialog] = useState<'new' | 'rename' | 'delete' | 'versions' | null>(null); const [title, setTitle] = useState(''); const [versions, setVersions] = useState<RuntimeInterfaceRevisionSummary[]>([]); const [dependencies, setDependencies] = useState<string[] | null>(null);
  const epoch = useRef(0); const request = useRef(new LibraryRequestFence()); const detailRequest = useRef(0); const operating = useRef(false); const currentSelected = useRef(selected); currentSelected.current = selected;
  const unavailable = runtimeUnavailable(native, status) || (!status?.capabilities?.features.interfaces ? 'This runtime has not acknowledged the interfaces capability.' : !workspaceId ? 'Open a workspace to access its authorized shared library.' : null);
  useEffect(() => { epoch.current++; setItems([]); setProposals([]); setSelected(null); setPreview(null); setVersions([]); setDialog(null); setError(''); setNotice(''); return () => { epoch.current++; }; }, [status?.generation, workspaceId]);
  const load = useCallback(async () => {
    if (unavailable || !workspaceId || operating.current) return;
    const sequence = request.current.issue(); const stamp = epoch.current; setLoading(true); setError('');
    try {
      const [library, pending] = await Promise.all([runtimeBridge.listInterfaces(workspaceId), runtimeBridge.listProposals(workspaceId)]);
      if (stamp !== epoch.current || !request.current.current(sequence)) return;
      setItems(library); setProposals(pending.filter(p => p.state === 'pending'));
      const previous = currentSelected.current;
      if (previous) {
        const newer = library.find(item => item.id === previous.id);
        if (newer && newer.revision !== previous.revision) { const detail = await runtimeBridge.getInterface(newer.id); if (stamp !== epoch.current || !request.current.current(sequence)) return; const checked = validateInterfaceSpec(detail.spec); if (checked.ok) setSelected(detail); else setError('Newer revision is invalid; previous valid view retained.'); }
        else if (!newer) { setSelected(null); setPreview(null); setNotice('This interface is no longer in the runtime library.'); }
      }
    } catch (failure) { if (stamp === epoch.current && request.current.current(sequence)) setError(libraryError(failure)); }
    finally { if (stamp === epoch.current && request.current.current(sequence)) setLoading(false); }
  }, [unavailable, workspaceId]);
  useEffect(() => { if (active) void load(); }, [active, load]);
  useEffect(() => {
    if (!active || !focusId || unavailable || operating.current) return;
    const stamp = epoch.current; const sequence = request.current.snapshot(); const detail = ++detailRequest.current;
    const current = () => stamp === epoch.current && request.current.current(sequence) && detail === detailRequest.current;
    void runtimeBridge.getInterface(focusId).then(item => { if (current()) { const result = validateInterfaceSpec(item.spec); if (result.ok) { setSelected(item); setPreview(null); } else setError(result.error); } }).catch(failure => { if (current()) setError(libraryError(failure)); });
    return () => { ++detailRequest.current; };
  }, [focusId, active, unavailable]);
  const act = async (action: () => Promise<void>) => { if (operating.current) return; operating.current = true; request.current.issue(); setLoading(false); setBusy(true); setError(''); setNotice(''); try { await action(); } catch (failure) { setError(`${libraryError(failure)} The last acknowledged view remains visible; remote mutation outcome may differ. Reload and compare before explicitly retrying.`); } finally { operating.current = false; setBusy(false); } };
  const accept = (item: RuntimeInterface) => { const result = validateInterfaceSpec(item.spec); if (!result.ok) throw new LibraryValidationError(result.error); setSelected(item); setItems(previous => [item, ...previous.filter(old => old.id !== item.id)]); setPreview(null); };
  const open = (summary: RuntimeInterfaceSummary) => void act(async () => { const item = await runtimeBridge.getInterface(summary.id); const result = validateInterfaceSpec(item.spec); if (!result.ok) throw new LibraryValidationError(result.error); setSelected(item); setPreview(null); });
  const showVersions = () => { if (!selected) return; setVersions([]); setDialog('versions'); void act(async () => { setVersions(await runtimeBridge.interfaceRevisions(selected.id)); }); };
  const showDelete = () => { if (!selected || !workspaceId) return; setDependencies(null); setDialog('delete'); void act(async () => { const schedules = await runtimeBridge.listSchedules(workspaceId); setDependencies(schedules.filter(s => s.interfaceId === selected.id).map(s => s.id)); }); };
  const submitDialog = () => void act(async () => {
    if (dialog === 'new' && workspaceId) { const draft = await runtimeBridge.proposeInterface({ workspaceId, expectedRevision: 0, title: title.trim(), spec: emptyInterfaceSpec() }); setPreview(draft); setSelected(null); setProposals(previous => [draft, ...previous]); setNotice('Empty draft saved as an unsaved proposal. No agent has run and nothing is published.'); }
    if (dialog === 'rename' && selected) accept(await runtimeBridge.renameInterface(selected.id, selected.revision, title.trim()));
    if (dialog === 'delete' && selected) { await runtimeBridge.deleteInterface(selected.id, selected.revision); setItems(previous => previous.filter(item => item.id !== selected.id)); setProposals(previous => previous.filter(p => p.interfaceId !== selected.id)); setSelected(null); setPreview(null); setNotice('Interface deleted. The runtime fences its dependent schedules and late publications.'); }
    setDialog(null);
  });
  const visible = items.filter(item => item.title.toLocaleLowerCase().includes(query.toLocaleLowerCase()));
  const viewed = preview ?? selected;
  return <section className="library-page" hidden={!active} aria-labelledby="interfaces-title" onKeyDown={listKeyboard}>
    <div className="library-heading"><div><span className="eyebrow">YOUR SHARED LIBRARY</span><h1 id="interfaces-title" tabIndex={-1}>{viewed?.title || 'Interfaces'}</h1><p>Persistent views, shared across your authorized conversations. Not public or team sharing.</p></div><div className="library-actions">{viewed && <button type="button" className="button secondary" onClick={() => { setSelected(null); setPreview(null); }}><ArrowLeft size={16} />All interfaces</button>}<button type="button" className="button secondary" disabled={loading || busy || !!unavailable} onClick={() => void load()}><ArrowClockwise size={16} />{loading ? 'Refreshing…' : 'Reload library'}</button><button type="button" className="button primary" disabled={busy || !!unavailable} onClick={() => { setTitle(''); setDialog('new'); }}><Plus size={16} />New interface</button></div></div>
    {unavailable && <div className="library-notice"><p>{unavailable}</p><button type="button" className="button secondary" onClick={onSettings}>Hermes status</button></div>}
    {error && <InlineError onRetry={() => void load()}>{error}</InlineError>}{notice && <p role="status" className="field-note">{notice}</p>}{loading && <p role="status" className="field-note">Loading runtime records. Previous valid view remains visible.</p>}
    {viewed ? <>
      <div className="interface-toolbar"><span className="status-chip">{preview ? 'Unsaved preview' : `Saved r${selected!.revision}`}</span><span className="field-note">Trusted components v1</span><div className="library-actions">
        {preview && <button type="button" className="button primary" disabled={busy || !!unavailable || !validateInterfaceSpec(preview.spec).ok} onClick={() => void act(async () => { const proposal = preview; accept(await runtimeBridge.publishInterface(proposal.id, proposal.expectedRevision)); setProposals(previous => previous.filter(item => item.id !== proposal.id)); setNotice('Published after runtime revision and source-run checks.'); })}>Publish preview</button>}
        <button type="button" className="button secondary" disabled={busy} onClick={() => onRevise(selected ? `Revise shared interface ${selected.id}, current revision ${selected.revision}, named “${selected.title}”. Inspect it and propose a new flexible trusted composition. Do not publish without my explicit action.` : `Create a flexible trusted interface named “${viewed.title}”. Propose the composition for my review; do not publish it.`)}>Revise in chat</button>
        {selected && <><button type="button" className="button secondary" disabled={busy || !!unavailable} onClick={showVersions}>Versions</button><button type="button" className="text-button" disabled={busy || !!unavailable} onClick={() => { setTitle(selected.title); setDialog('rename'); }}>Rename</button><button type="button" className="text-button danger-text" disabled={busy || !!unavailable} onClick={showDelete}>Delete</button></>}
      </div></div>
      {preview && <p className="field-note">Proposal {preview.id} · {preview.sourceRunId ? `Hermes run ${preview.sourceRunId}` : 'Operator-created empty draft'} · expected revision {preview.expectedRevision}. Preview is not saved publication.</p>}
      <InterfaceRenderer spec={viewed.spec} />
      <div className="library-notice"><p>Live-data refresh is unavailable until scoped Google reads and retention are verified. Reload library only retrieves saved presentation records.</p><button type="button" className="button secondary" disabled>Refresh account data — unavailable</button></div>
    </> : <>
      <label className="library-search">Find an interface<input data-library-search aria-label="Search interfaces" value={query} onChange={event => setQuery(event.target.value)} placeholder="Search by name…" /></label>
      <div className="library-table-scroll"><table className="library-table"><thead><tr><th>Name</th><th>Revision</th><th>Last saved</th><th>Data source</th></tr></thead><tbody>{visible.map(item => <tr key={item.id}><td><button type="button" data-library-row className="library-row-button" disabled={busy} onClick={() => open(item)}>{item.title}<span>Trusted components</span></button></td><td>r{item.revision}</td><td><time dateTime={item.updatedAt}>{new Date(item.updatedAt).toLocaleString()}</time></td><td>Conversation · not live</td></tr>)}</tbody></table></div>
      {!visible.length && !loading && <div className="library-empty"><h2>{query ? 'No matching interfaces' : 'No interfaces yet'}</h2><p>{query ? 'Try a shorter name. Your saved library is unchanged.' : 'Create a persistent view from a conversation. Start with an empty draft, then ask Hermes for a composition.'}</p>{query && <button type="button" className="button secondary" onClick={() => setQuery('')}>Clear search</button>}</div>}
    </>}
    {proposals.length > 0 && <section className="proposal-section"><h2>Proposals awaiting your review</h2><div className="proposal-list">{proposals.map(proposal => <button type="button" className="proposal-row" key={proposal.id} disabled={busy} onClick={() => void act(async () => { const detail = await runtimeBridge.getProposal(proposal.id); const result = validateInterfaceSpec(detail.spec); if (!result.ok) throw new LibraryValidationError(result.error); const saved = detail.interfaceId ? await runtimeBridge.getInterface(detail.interfaceId) : null; if (saved && !validateInterfaceSpec(saved.spec).ok) throw new LibraryValidationError('Saved revision is invalid. Previous view retained.'); setPreview(detail); setSelected(saved); })}><span>{proposal.title}<small>{proposal.interfaceId ? `Revision proposal · based on r${proposal.expectedRevision}` : 'New interface proposal'} · Not published</small></span><span>Preview</span></button>)}</div></section>}
    {dialog && <Modal title={dialog === 'new' ? 'New interface draft' : dialog === 'rename' ? 'Rename interface' : dialog === 'delete' ? 'Delete this interface?' : 'Immutable version history'} onClose={() => setDialog(null)} dismissible={!busy} className="library-modal">
      {dialog === 'versions' ? <><p className="field-note">Rollback creates a new revision from a previous presentation. It does not restore permissions or expired data.</p><div className="version-list">{versions.map(version => <div className="version-row" key={version.revision}><span>r{version.revision} · {version.title}<small>{version.createdAt}</small></span><button type="button" className="button secondary" disabled={busy || version.revision === selected?.revision} onClick={() => void act(async () => { accept(await runtimeBridge.rollbackInterface(selected!.id, selected!.revision, version.revision)); setDialog(null); setNotice(`Previous presentation restored as a new revision from r${version.revision}.`); })}>Restore as new revision</button></div>)}</div>{!versions.length && <p className="field-note">{busy ? 'Loading versions…' : 'No version records returned.'}</p>}</> : <form onSubmit={event => { event.preventDefault(); submitDialog(); }} onKeyDown={event => { if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') { event.preventDefault(); event.currentTarget.requestSubmit(); } }}>
        {dialog === 'delete' ? <><p className="dialog-description">Permanently delete “{selected?.title}”? This separately deletes the shared interface, not its conversations. Dependent schedules are paused and in-flight publications fenced by the runtime. No undo is available.</p><p className="field-note">{dependencies === null ? 'Checking visible schedule dependencies…' : `${dependencies.length} dependent schedule(s) visible in this workspace. Dependencies in other authorized workspaces are also fenced by the runtime.`}</p></> : <label>Interface name<input autoFocus value={title} maxLength={160} required disabled={busy} onChange={event => setTitle(event.target.value)} /></label>}
        <div className="modal-actions"><button type="button" className="button secondary" disabled={busy} onClick={() => setDialog(null)}>Cancel</button><button type="submit" className={`button ${dialog === 'delete' ? 'danger' : 'primary'}`} disabled={busy || (dialog === 'delete' ? dependencies === null : !title.trim())}>{busy ? 'Saving…' : dialog === 'delete' ? 'Delete interface' : dialog === 'new' ? 'Create empty draft' : 'Save name'}</button></div>
      </form>}{error && <InlineError>{error}</InlineError>}
    </Modal>}
  </section>;
}
