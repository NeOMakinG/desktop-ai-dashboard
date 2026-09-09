import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState, type FormEvent, type ReactNode } from 'react';
import { AnimatePresence, MotionConfig, motion, useReducedMotion } from 'motion/react';
import { isTauri } from '@tauri-apps/api/core';
import { ArrowDown, ArrowLeft, ArrowRight, ArrowUp, ArrowUUpLeft, BookmarkSimple, CalendarBlank, CaretDown, CaretRight, Check, CheckCircle, Clock, Command, EnvelopeSimple, GridFour, Info, MagnifyingGlass, Moon, Plus, SidebarSimple, SlidersHorizontal, Sparkle, SquaresFour, Sun, Tray, X } from '@phosphor-icons/react';
import { INITIAL_WORKSPACE, MEETINGS, MESSAGES, TRAVEL_MEETINGS, TRAVEL_MESSAGES, WORKSPACE_COPY, classifyPrompt, type FixtureItem, type Variant, type Workspace } from './fixtures';
import './prototype.css';

type ChatMessage = { id: number; role: 'assistant' | 'user'; text: string };
type Snapshot = { id: number; workspace: Workspace; name: string };
type DialogKind = 'palette' | 'connections' | 'versions' | 'inspector' | 'restore' | 'shortcuts' | null;
type PaletteAction = { group: string; label: string; icon: ReactNode; action: () => void; disabled?: boolean };
const VARIANTS: Variant[] = ['studio', 'focus', 'canvas'];
const MAX_PROMPTS = 30;
const INITIAL_CHAT: ChatMessage[] = [{ id: 0, role: 'assistant', text: 'A workspace that takes the shape of your day. I’ve arranged a little sample to get us started. What would make it yours?' }];
const cloneWorkspace = (value: Workspace): Workspace => ({ ...value, notes: { ...value.notes }, done: [...value.done] });
const initialVariant = (): Variant => {
  const candidate = new URLSearchParams(window.location.search).get('variant');
  return VARIANTS.includes(candidate as Variant) ? candidate as Variant : 'studio';
};
const isInteractive = (target: EventTarget | null) => target instanceof Element && !!target.closest('input, textarea, select, button, a, summary, [contenteditable]:not([contenteditable="false"]), [role], [tabindex], audio, video, iframe');

function FormaMark({ small = false }: { small?: boolean }) {
  return <img className={small ? 'forma-mark small' : 'forma-mark'} src="/forma.svg" alt="" draggable={false} />;
}
function IconButton({ label, children, onClick, className = '', disabled = false }: { label: string; children: ReactNode; onClick: () => void; className?: string; disabled?: boolean }) {
  return <button type="button" className={`icon-button ${className}`} aria-label={label} title={label} onClick={onClick} disabled={disabled}>{children}</button>;
}
function Modal({ title, children, onClose, className = '' }: { title: string; children: ReactNode; onClose: () => void; className?: string }) {
  const ref = useRef<HTMLDialogElement>(null);
  const labelId = useId();
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  useLayoutEffect(() => {
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const dialog = ref.current;
    dialog?.showModal();
    return () => {
      dialog?.close();
      if (previous?.isConnected) previous.focus();
    };
  }, []);
  return <dialog ref={ref} className={`modal ${className}`} aria-labelledby={labelId} onCancel={event => { event.preventDefault(); closeRef.current(); }} onClick={event => { if (event.target === event.currentTarget) { const rect = event.currentTarget.getBoundingClientRect(); if (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom) onClose(); } }}>
    <div className="modal-header"><h2 id={labelId}>{title}</h2><IconButton label="Close dialog" onClick={onClose}><X size={19} /></IconButton></div>
    {children}
  </dialog>;
}

type ComposerProps = { prompt: string; setPrompt: (value: string) => void; onSubmit: (value: string) => void; busy: boolean; onCancel: () => void; error: string; exhausted: boolean; large?: boolean };
function Composer({ prompt, setPrompt, onSubmit, busy, onCancel, error, exhausted, large = false }: ComposerProps) {
  const id = useId();
  const submit = (event: FormEvent) => { event.preventDefault(); onSubmit(prompt); };
  return <div className={`composer-wrap ${large ? 'large' : ''}`}>
    <form className="composer" onSubmit={submit}>
      <label className="sr-only" htmlFor={id}>Describe your workspace</label>
      <textarea id={id} value={prompt} maxLength={500} rows={large ? 2 : 3} placeholder={large ? 'What would a calmer day look like?' : 'Make a little room for what matters…'} onChange={event => setPrompt(event.target.value)} onKeyDown={event => { if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); if (!busy) onSubmit(prompt); } }} aria-describedby={`${id}-hint`} disabled={exhausted} />
      <div className="composer-actions"><span className="composer-mode"><Sparkle size={14} /> Scripted preview</span>{busy ? <button className="cancel-button" type="button" onClick={onCancel}><span className="stop-square" /> Cancel</button> : <button className="send-button" type="submit" disabled={exhausted} aria-label="Arrange scripted workspace" title="Arrange scripted workspace · Enter"><ArrowUp size={18} /></button>}</div>
    </form>
    <p id={`${id}-hint`} className="composer-disclosure">Scripted preview · no model connected</p>
    {(error || exhausted) && <p className="inline-error" role="alert">{exhausted ? 'This session has reached 30 prompts. Reset the demo from the command menu to keep exploring.' : error}</p>}
  </div>;
}
function ChatPanel({ chat, composer, onExample, busy, close }: { chat: ChatMessage[]; composer: ComposerProps; onExample: (value: string) => void; busy: boolean; close?: () => void }) {
  const bottomRef = useRef<HTMLDivElement>(null);
  useEffect(() => { bottomRef.current?.scrollIntoView({ block: 'nearest', behavior: 'instant' }); }, [chat.length, busy]);
  return <section className="chat-panel" aria-label="Shape your workspace">
    <div className="chat-heading"><span><Sparkle size={17} /> Shape your day</span>{close && <IconButton label="Hide conversation" onClick={close}><X size={16} /></IconButton>}</div>
    <div className="chat-scroll">
      <div className="conversation-label">A little conversation.<br />A space that feels like you.</div>
      {chat.map((message, index) => <motion.div key={message.id} className={`chat-message ${message.role}`} initial={index === 0 ? false : { opacity: 0 }} animate={{ opacity: 1 }}>
        {message.role === 'assistant' && <div className="assistant-byline"><FormaMark small /><span>Forma <span className="tiny-label">demo</span></span></div>}
        <p>{message.text}</p>
        {index === 0 && <div className="created-workspace"><div className="mini-workspace-icon"><SquaresFour size={18} /></div><span>Daily workspace<small>A starting point, just for this demo</small></span><Check size={14} /></div>}
      </motion.div>)}
      {busy && <p className="chat-progress"><span className="static-progress-dot" /> Arranging the scripted preview…</p>}
      <div ref={bottomRef} />
    </div>
    <div className="chat-bottom"><p className="try-label">A few things to try</p><div className="chat-suggestions"><button disabled={busy} onClick={() => onExample('Bring my inbox into focus')}><Tray size={15} /> Bring my inbox into focus <ArrowUp size={13} /></button><button disabled={busy} onClick={() => onExample('Protect time before the review')}><Moon size={15} /> Make room for deep work <ArrowUp size={13} /></button></div><Composer {...composer} /></div>
  </section>;
}

type WorkspaceViewProps = {
  workspace: Workspace; query: string; setQuery: (query: string) => void; filter: string; setFilter: (filter: string) => void;
  onSelect: (item: FixtureItem) => void; onRefine: (kind: 'unread' | 'quiet') => void; busy: boolean; stage: number;
  error: boolean; onRetry: () => void; onDismiss: () => void; density: boolean; onDensity: () => void; minimalHero?: boolean;
};
function WorkspaceView({ workspace, query, setQuery, filter, setFilter, onSelect, onRefine, busy, stage, error, onRetry, onDismiss, density, onDensity, minimalHero = false }: WorkspaceViewProps) {
  const copy = WORKSPACE_COPY[workspace.kind];
  const travel = workspace.kind === 'travel';
  const sourceMessages = travel ? TRAVEL_MESSAGES : MESSAGES;
  const meetings = travel ? TRAVEL_MEETINGS : MEETINGS;
  const messages = sourceMessages.filter(item => (filter !== 'reply' || item.needsReply) && (!workspace.unread || item.needsReply) && `${item.person} ${item.title} ${item.preview}`.toLowerCase().includes(query.toLowerCase().trim()));
  const searchId = useId();
  return <article className={`workspace-document ${workspace.quiet ? 'quiet-workspace' : ''} ${density ? 'compact' : ''} ${minimalHero ? 'embedded-document' : ''}`} aria-label={copy.label}>
    {!minimalHero && <header className="workspace-hero"><div className="day-eyebrow"><span className="date-dot" />{travel ? 'Thursday, September 10' : 'Wednesday, September 9'}<span className="demo-date">Sample day</span></div><h1>{copy.title}</h1><p>{copy.subtitle}</p></header>}
    <AnimatePresence mode="wait">
      {busy && <motion.div key="progress" initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }} className="generation-strip" role="status"><Sparkle size={15} /><span>Scripted demo · {stage === 1 ? 'finding a little clarity' : stage === 2 ? 'arranging your workspace' : 'putting things in place'}</span><span className="progress-track"><span style={{ width: `${stage * 30}%` }} /></span></motion.div>}
    </AnimatePresence>
    {error && <div className="failure-banner" role="alert"><Info size={18} /><div>The demo preview could not be arranged.<small>Your previous version is unchanged.</small></div><button onClick={onRetry}>Retry</button><IconButton label="Dismiss preview error" onClick={onDismiss}><X size={16} /></IconButton></div>}
    <motion.div className="workspace-content" key={`${workspace.kind}-${workspace.quiet}`} initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ duration: 0.25 }}>
      <section className="daily-brief" aria-labelledby="brief-title"><div className="brief-symbol"><Sun size={26} /></div><div><div className="section-eyebrow" id="brief-title">The short version</div><p>{copy.brief}</p></div><span className="brief-sparkle" aria-hidden="true"><Sparkle size={21} /></span></section>
      {workspace.kind === 'focus' && <section className="protected-time"><div className="time-block-icon"><Moon size={22} /></div><div><h2>A little time, just for the review.</h2><p>13:30 – 13:55 <span>·</span> 25 minutes of undisturbed preparation</p><small>Suggestion · not added to calendar</small></div></section>}
      <div className="workspace-sections">
        <section className="inbox-section" aria-labelledby="inbox-title"><div className="section-heading"><div><h2 id="inbox-title">{travel ? 'Before you go' : 'Worth your attention'} <span className="section-count">{messages.length}</span></h2><p>{travel ? 'The useful details, all together.' : 'Good conversations. A clear next step.'}</p></div><IconButton label={density ? 'Use comfortable density' : 'Use compact density'} onClick={onDensity}><SlidersHorizontal size={18} /></IconButton></div>
          <div className="inbox-toolbar"><div className="filter-tabs" role="group" aria-label="Filter messages"><button className={filter === 'all' ? 'active' : ''} aria-pressed={filter === 'all'} onClick={() => setFilter('all')}>All messages</button><button className={filter === 'reply' ? 'active' : ''} aria-pressed={filter === 'reply'} onClick={() => setFilter('reply')}>Needs reply<span className="filter-dot" /></button></div><div className="search-field"><MagnifyingGlass size={15} /><label className="sr-only" htmlFor={searchId}>Search messages</label><input id={searchId} data-workspace-search value={query} onChange={event => setQuery(event.target.value.slice(0, 100))} placeholder="Search" maxLength={100} />{query && <button aria-label="Clear search" onClick={() => setQuery('')}><X size={13} /></button>}</div></div>
          <div className="message-list">{messages.map(item => <button className={`message-row ${workspace.done.includes(item.id) ? 'is-done' : ''}`} key={item.id} onClick={() => onSelect(item)} aria-label={`Open message from ${item.person}: ${item.title}`}><span className="initials-avatar">{item.initials}</span><span className="message-content"><span className="message-person">{item.person}{item.needsReply && <span className="unread-dot" aria-label="Needs reply" />}</span><span className="message-title">{item.title}</span><span className="message-preview">{item.preview}</span></span><span className="message-meta"><time title={`2026-09-09T${item.time}:00+02:00`}>{item.time}</time>{workspace.done.includes(item.id) ? <CheckCircle size={17} aria-label="Done in demo" /> : <CaretRight size={14} />}</span></button>)}
          {!messages.length && <div className="empty-filter"><MagnifyingGlass size={25} /><h3>No messages in this view.</h3><p>{query ? <>Nothing matches “{query}”. Try another name or subject.</> : 'Try showing all of the sample conversations.'}</p><button className="secondary-button" onClick={() => { setQuery(''); setFilter('all'); if (workspace.unread) onRefine('unread'); }}>Clear filters</button></div>}</div>
          <div className="section-source"><EnvelopeSimple size={13} />{travel ? 'Sample travel messages' : 'Sample messages'}<span>Nothing sent. Nothing connected.</span></div>
        </section>
        <div className="lower-sections"><section className="agenda-section" aria-labelledby="agenda-title"><div className="section-heading"><div><h2 id="agenda-title">{travel ? 'On the itinerary' : 'A rhythm for your day'}</h2><p>{travel ? 'Thursday, September 10' : 'A little structure. Plenty of possibility.'}</p></div><CalendarBlank size={19} className="muted-icon" /></div><div className="agenda-list">{meetings.map((item, index) => <button className={`agenda-row ${index === 0 ? 'next-meeting' : ''}`} key={item.id} onClick={() => onSelect(item)}><time>{item.time}<small>{item.end}</small></time><span className="agenda-line" /><span className="agenda-text"><strong>{item.title}</strong><small>{item.person}</small></span>{index === 0 && <span className="next-tag">{travel ? 'First stop' : 'Up next'}</span>}</button>)}</div><div className="section-source"><CalendarBlank size={13} />{travel ? 'Sample itinerary · no bookings' : 'Sample calendar'}</div></section>
        <aside className="preparation-note"><div className="note-top"><span className="note-icon"><BookmarkSimple size={18} /></span><span>A thought for later</span></div><h3>{travel ? 'Enjoy the in-between.' : workspace.kind === 'focus' ? 'Protect your headspace.' : 'Arrive with a clear head.'}</h3><p>{copy.note}</p><div className="note-bottom"><span>{travel ? 'Sample travel note' : 'For your 14:00 review'}</span><ArrowDown size={15} /></div></aside></div>
      </div>
      <div className="refinement-bar"><span><Sparkle size={14} /> Make it more you</span><button disabled={busy} aria-pressed={workspace.unread} onClick={() => onRefine('unread')}>{workspace.unread ? 'Show every message' : 'Only what needs a reply'}<Plus size={13} /></button><button disabled={busy} aria-pressed={workspace.quiet} onClick={() => onRefine('quiet')}>{workspace.quiet ? 'Bring back the details' : 'A quieter layout'}<Plus size={13} /></button></div>
      <footer className="workspace-footer"><span>Sample data · frozen at 09:12 CEST</span><span>Made of possibilities, not connected accounts.</span></footer>
    </motion.div>
  </article>;
}

function InspectorContent({ item, workspace, onNote, onDone }: { item: FixtureItem; workspace: Workspace; onNote: (value: string) => void; onDone: () => void }) {
  const [note, setNote] = useState(workspace.notes[item.id] ?? '');
  const changed = note !== (workspace.notes[item.id] ?? '');
  return <div className="inspector-content"><div className="source-badge">{item.type === 'message' ? <EnvelopeSimple size={15} /> : <CalendarBlank size={15} />}{item.source}</div><h3>{item.title}</h3><div className="inspector-person"><span className="initials-avatar">{item.initials}</span><div><strong>{item.person}</strong><span>{item.type === 'message' ? 'Wednesday, September 9' : item.source.includes('Thursday') ? 'Thursday, September 10' : 'Wednesday, September 9'} · {item.time}{item.end ? `–${item.end}` : ''}</span></div></div><p className="inspector-body">{item.body}</p><div className="inspector-divider" /><label className="note-label" htmlFor="preparation-note">{item.type === 'meeting' ? 'My preparation' : 'A note to myself'}<span>Only in this demo session</span></label><textarea id="preparation-note" className="note-editor" value={note} maxLength={1000} placeholder="Leave yourself a little context…" onChange={event => setNote(event.target.value)} onKeyDown={event => { if ((event.metaKey || event.ctrlKey) && event.key === 'Enter') { event.preventDefault(); onNote(note); } }} /><div className="note-editor-footer"><span>{note.length}/1,000</span><button className="secondary-button" onClick={() => onNote(note)} disabled={!changed}>Keep note</button></div><button className="done-button" onClick={onDone}><CheckCircle size={18} />{workspace.done.includes(item.id) ? 'Marked done · undo' : 'Mark done in demo'}</button><p className="inspector-truth"><Info size={15} />Synthetic content. No message is sent, no event is changed, and nothing leaves this window.</p>{changed && <p className="note-unsaved">Keep your note before closing to retain it.</p>}</div>;
}

export function PrototypeApp() {
  const reducedMotion = useReducedMotion();
  const [variant, setVariant] = useState<Variant>(initialVariant);
  const [workspace, setWorkspace] = useState<Workspace>(() => cloneWorkspace(INITIAL_WORKSPACE));
  const [saved, setSaved] = useState<Snapshot[]>([]);
  const [page, setPage] = useState<'today' | 'saved'>('today');
  const [chat, setChat] = useState<ChatMessage[]>(INITIAL_CHAT);
  const [prompt, setPrompt] = useState('');
  const [promptError, setPromptError] = useState('');
  const [promptCount, setPromptCount] = useState(0);
  const [busy, setBusy] = useState(false);
  const [stage, setStage] = useState(0);
  const [previewError, setPreviewError] = useState(false);
  const [toast, setToast] = useState('');
  const [dialog, setDialog] = useState<DialogKind>(null);
  const [selected, setSelected] = useState<FixtureItem | null>(null);
  const [restoreTarget, setRestoreTarget] = useState<Snapshot | null>(null);
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState('all');
  const [density, setDensity] = useState(false);
  const [canvasChat, setCanvasChat] = useState(false);
  const [paletteQuery, setPaletteQuery] = useState('');
  const [blank, setBlank] = useState(false);
  const timers = useRef<ReturnType<typeof setTimeout>[]>([]);
  const toastTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const generation = useRef(0);
  const busyRef = useRef(false);
  const ids = useRef(1);
  const revision = useRef(1);
  const retryPrompt = useRef('Arrange my day');
  const native = isTauri();
  const copy = WORKSPACE_COPY[workspace.kind];
  const savedCurrent = saved.some(snapshot => JSON.stringify(snapshot.workspace) === JSON.stringify(workspace));
  const notify = useCallback((text: string) => { if (toastTimer.current) clearTimeout(toastTimer.current); setToast(text); toastTimer.current = setTimeout(() => setToast(''), 4500); }, []);
  const stopGeneration = useCallback(() => {
    generation.current += 1;
    timers.current.forEach(clearTimeout);
    timers.current = [];
    busyRef.current = false;
    setBusy(false);
    setStage(0);
  }, []);
  useEffect(() => () => { timers.current.forEach(clearTimeout); if (toastTimer.current) clearTimeout(toastTimer.current); }, []);
  const cancel = () => { if (!busyRef.current) return; stopGeneration(); setChat(previous => [...previous, { id: ids.current++, role: 'assistant' as const, text: 'Preview cancelled. Your last completed workspace is unchanged.' }].slice(-12)); notify('Cancelled · your workspace is unchanged'); };
  const changeVariant = (next: Variant) => {
    if (next === variant) return;
    stopGeneration();
    setVariant(next);
    setDialog(null);
    const url = new URL(window.location.href);
    url.searchParams.set('variant', next);
    window.history.replaceState(null, '', url);
  };
  useEffect(() => {
    const url = new URL(window.location.href);
    if (url.searchParams.get('variant') !== variant) { url.searchParams.set('variant', variant); window.history.replaceState(null, '', url); }
  }, [variant]);
  const addChat = (messages: Omit<ChatMessage, 'id'>[]) => setChat(previous => [...previous, ...messages.map(message => ({ ...message, id: ids.current++ }))].slice(-12));
  const runPreview = (text: string, refinement?: 'unread' | 'quiet', fail = false) => {
    if (busyRef.current) return;
    const clean = text.trim().slice(0, 500);
    if (!clean) { setPromptError('Describe a workspace, or choose an example.'); return; }
    if (promptCount >= MAX_PROMPTS && !refinement && !fail) { setPromptError('Demo prompt limit reached. Reset the demo to keep exploring.'); return; }
    setPromptError(''); setPreviewError(false); setPage('today');
    busyRef.current = true; setBusy(true); setStage(1);
    const token = ++generation.current;
    const match = classifyPrompt(clean);
    const target = cloneWorkspace(workspace);
    if (refinement) target[refinement] = !target[refinement];
    else { target.kind = match.kind; target.quiet = match.kind === 'focus'; target.unread = match.kind === 'focus'; }
    retryPrompt.current = clean;
    if (!refinement && !fail) { setPromptCount(count => count + 1); setPrompt(''); }
    addChat([{ role: 'user', text: clean }, { role: 'assistant', text: fail ? 'Trying a simulated preview failure. Your current space stays safe.' : refinement ? 'A small adjustment, a little more breathing room. Arranging a scripted refinement.' : match.supported ? `I’ll arrange a ${match.kind === 'daily' ? 'daily' : match.kind} space using sample content. This is a scripted layout, not a model response.` : 'This demo uses scripted layouts, so that request is not supported. Here is the closest preview: a calm daily workspace.' }]);
    const schedule = (delay: number, fn: () => void) => { timers.current.push(setTimeout(() => { if (generation.current === token && busyRef.current) fn(); }, reducedMotion ? Math.min(delay / 20, 80) : delay)); };
    schedule(400, () => setStage(2)); schedule(850, () => setStage(3));
    schedule(1450, () => {
      timers.current = [];
      busyRef.current = false; setBusy(false); setStage(0);
      if (fail) { setPreviewError(true); notify('Simulated error · your previous workspace is unchanged'); return; }
      target.revision = ++revision.current;
      setWorkspace(target); setBlank(false);
      if (!refinement) { setQuery(''); setFilter('all'); }
      notify('Demo preview ready · arranged with sample content');
    });
  };
  const refine = (kind: 'unread' | 'quiet') => runPreview(kind === 'unread' ? workspace.unread ? 'Show every message' : 'Only show messages that need a reply' : workspace.quiet ? 'Bring back the details' : 'Make the layout quieter', kind);
  const saveWorkspace = () => {
    if (busyRef.current || previewError || blank) return;
    if (savedCurrent) { notify('This version is already saved · session only'); return; }
    if (saved.length >= 8) { notify('Eight session versions saved. Reset the demo to start a fresh collection.'); return; }
    setSaved(previous => [...previous, { id: ids.current++, workspace: cloneWorkspace(workspace), name: copy.label }]);
    notify('Saved in this demo · resets on reload');
  };
  const restore = (snapshot: Snapshot) => {
    stopGeneration(); setWorkspace(cloneWorkspace(snapshot.workspace)); setBlank(false); setPage('today'); setDialog(null); setSelected(null); setQuery(''); setFilter('all'); setPreviewError(false);
    notify(`Restored version ${snapshot.workspace.revision} · later saves are still here`);
  };
  const requestRestore = (snapshot: Snapshot) => {
    if (!savedCurrent) { setRestoreTarget(snapshot); setDialog('restore'); }
    else restore(snapshot);
  };
  const reset = () => {
    stopGeneration(); setWorkspace(cloneWorkspace(INITIAL_WORKSPACE)); setSaved([]); setChat(INITIAL_CHAT); setPrompt(''); setPromptCount(0); setPromptError(''); setPreviewError(false); setQuery(''); setFilter('all'); setSelected(null); setDialog(null); setPage('today'); setBlank(false); revision.current = 1; notify('Fresh start · all session-only changes cleared');
  };
  const openPalette = () => { setPaletteQuery(''); setDialog('palette'); };
  const selectItem = (item: FixtureItem) => { setSelected(item); setDialog('inspector'); };
  const editWorkspace = (edit: (value: Workspace) => Workspace) => {
    stopGeneration(); setWorkspace(current => ({ ...edit(cloneWorkspace(current)), revision: ++revision.current })); setPreviewError(false);
  };
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') { event.preventDefault(); if (dialog === 'palette') setDialog(null); else if (!dialog) openPalette(); return; }
      if (dialog) return;
      if (isInteractive(event.target) || event.altKey || event.metaKey || event.ctrlKey) return;
      if (event.key === '?') { event.preventDefault(); setDialog('shortcuts'); return; }
      if (event.shiftKey) return;
      if (import.meta.env.DEV && (event.key === 'ArrowLeft' || event.key === 'ArrowRight')) { event.preventDefault(); changeVariant(VARIANTS[(VARIANTS.indexOf(variant) + (event.key === 'ArrowRight' ? 1 : 2)) % 3]); }
      if (event.key === '/') { const input = document.querySelector<HTMLInputElement>('[data-workspace-search]'); if (input) { event.preventDefault(); input.focus(); } }
      if (event.key === '?') { event.preventDefault(); setDialog('shortcuts'); }
    };
    document.addEventListener('keydown', onKeyDown);
    return () => document.removeEventListener('keydown', onKeyDown);
  });
  const composerProps: ComposerProps = { prompt, setPrompt, onSubmit: value => runPreview(value), busy, onCancel: cancel, error: promptError, exhausted: promptCount >= MAX_PROMPTS };
  const viewProps: WorkspaceViewProps = { workspace, query, setQuery, filter, setFilter, onSelect: selectItem, onRefine: refine, busy, stage, error: previewError, onRetry: () => runPreview(retryPrompt.current), onDismiss: () => setPreviewError(false), density, onDensity: () => setDensity(value => !value) };
  const paletteActions: PaletteAction[] = [
    { group: 'Navigate', label: 'Today’s workspace', icon: <Sun size={18} />, action: () => setPage('today') },
    { group: 'Navigate', label: 'Saved spaces · session only', icon: <BookmarkSimple size={18} />, action: () => setPage('saved') },
    { group: 'Workspace', label: 'Save current version to memory', icon: <Plus size={18} />, action: saveWorkspace, disabled: busy || previewError || blank },
    { group: 'Workspace', label: 'Version history and rollback', icon: <ArrowUUpLeft size={18} />, action: () => setDialog('versions') },
    { group: 'Workspace', label: 'Arrange a sample travel workspace', icon: <SquaresFour size={18} />, action: () => runPreview('Plan my trip to Lyon'), disabled: busy },
    ...VARIANTS.map(layout => ({ group: 'Layouts', label: `${layout.charAt(0).toUpperCase() + layout.slice(1)} layout${layout === variant ? ' · current' : ''}`, icon: <SquaresFour size={18} />, action: () => changeVariant(layout) })),
    { group: 'Settings', label: density ? 'Use comfortable density' : 'Use compact density', icon: <SlidersHorizontal size={18} />, action: () => setDensity(value => !value) },
    { group: 'Settings', label: 'Connections · none connected', icon: <EnvelopeSimple size={18} />, action: () => setDialog('connections') },
    { group: 'Settings', label: 'Keyboard shortcuts', icon: <Command size={18} />, action: () => setDialog('shortcuts') },
    { group: 'Demo', label: 'Simulate preview error', icon: <Info size={18} />, action: () => runPreview('Arrange my day', undefined, true), disabled: busy },
    { group: 'Demo', label: 'Start with a blank workspace', icon: <GridFour size={18} />, action: () => { stopGeneration(); setBlank(true); setPage('today'); setPreviewError(false); } },
    { group: 'Demo', label: 'Reset demo · clear session changes', icon: <ArrowUUpLeft size={18} />, action: reset },
  ].filter(action => action.label.toLowerCase().includes(paletteQuery.toLowerCase()));

  return <MotionConfig reducedMotion="user" transition={{ type: 'spring', stiffness: 330, damping: 32 }}>
    <div className={`prototype-app variant-${variant} ${density ? 'density-compact' : ''} ${native ? 'is-native' : 'is-browser'}`}>
      <div className="window-chrome" data-tauri-drag-region onContextMenu={event => { if (native) event.preventDefault(); }} aria-hidden="true">{!native && <div className="traffic-lights"><span /><span /><span /></div>}</div>
      {variant === 'studio' && <aside className="sidebar"><div className="sidebar-brand"><FormaMark /><span>forma</span></div><div className="sidebar-content"><button className="new-space-button" onClick={() => { setPage('today'); document.querySelector<HTMLTextAreaElement>('.composer textarea')?.focus(); }}><Plus size={17} /> A new possibility<span>⌘ K</span></button><p className="nav-label">Your workspace</p><nav aria-label="Main navigation"><button className={page === 'today' ? 'nav-item active' : 'nav-item'} onClick={() => setPage('today')}><Sun size={19} /><span>Today</span><span className="nav-active-dot" /></button><button className={page === 'saved' ? 'nav-item active' : 'nav-item'} onClick={() => setPage('saved')}><SquaresFour size={19} /><span>Saved spaces</span>{saved.length > 0 && <span className="nav-count">{saved.length}</span>}</button></nav><div className="sources-group"><p className="nav-label">A little context</p><button onClick={() => setDialog('connections')}><EnvelopeSimple size={18} /><span>Messages</span><span className="sample-tag">Sample</span></button><button onClick={() => setDialog('connections')}><CalendarBlank size={18} /><span>Calendar</span><span className="sample-tag">Sample</span></button><button className="add-source" onClick={() => setDialog('connections')}><Plus size={16} /> About connections</button></div></div><div className="sidebar-bottom"><div className="sidebar-note"><span className="small-orbit"><Sparkle size={17} /></span><p>A little less noise.<br /><strong>A little more you.</strong></p></div><button className="demo-info-button" onClick={() => setDialog('connections')}><Info size={16} /><span>This demo<span>Nothing connected</span></span><CaretRight size={13} /></button></div></aside>}
      <div className="main-shell"><header className="topbar"><div className="topbar-left">{variant !== 'studio' && <button className="header-brand" onClick={() => setPage('today')} aria-label="Forma, go to today"><FormaMark small /><span>forma</span></button>}<span className="topbar-title">{page === 'saved' ? 'Saved spaces' : copy.label}</span><span className={`revision-status ${savedCurrent ? 'saved' : ''}`}>{savedCurrent ? <Check size={11} /> : <span />}{savedCurrent ? 'Saved' : 'Draft'}<span className="revision-number">v{workspace.revision}</span></span></div><div className="topbar-actions"><span className="trust-label">Interactive demo · no accounts connected</span><IconButton label="Command menu · ⌘ / Ctrl K" onClick={openPalette}><Command size={17} /></IconButton><button className="versions-button" onClick={() => setDialog('versions')} title="Version history and restore"><Clock size={16} /><span>Versions</span><CaretDown size={11} /></button><button className="save-button" onClick={saveWorkspace} disabled={busy || previewError || blank} title="Save a snapshot in this session only">{savedCurrent ? <Check size={15} /> : <BookmarkSimple size={15} />}<span>{savedCurrent ? 'Saved to memory' : 'Save to memory'}</span></button></div></header>
        {page === 'saved' ? <main className="saved-page"><div className="saved-page-heading"><span className="section-eyebrow">A collection of possibilities</span><h1>Spaces worth keeping.</h1><p>Little arrangements that work for you. Saved only in this session.</p></div>{saved.length ? <div className="saved-gallery">{saved.map(snapshot => <button className="saved-card" key={snapshot.id} onClick={() => requestRestore(snapshot)}><div className={`saved-miniature miniature-${snapshot.workspace.kind}`}><div className="miniature-title" /><div className="miniature-brief" /><div className="miniature-row" /><div className="miniature-row" /><div className="miniature-columns"><span /><span /></div></div><div className="saved-card-label"><span><strong>{snapshot.name}</strong><small>Version {snapshot.workspace.revision} · session only</small></span><ArrowUp size={18} /></div></button>)}</div> : <div className="saved-empty"><BookmarkSimple size={38} /><h2>A place for your favorite arrangements.</h2><p>Save a workspace to find it here. Closing or reloading this window clears the collection.</p><button className="primary-button" onClick={() => { setPage('today'); setBlank(false); }}>Create a workspace <ArrowRight size={16} /></button></div>}<button className="text-button" onClick={() => setPage('today')}><ArrowLeft size={16} /> Back to today</button></main> : <div className="working-area">
          {variant === 'studio' && <ChatPanel chat={chat} composer={composerProps} onExample={value => runPreview(value)} busy={busy} />}
          <main className="workspace-scroll">
            {variant === 'focus' && <section className="focus-welcome"><div className="focus-symbol"><FormaMark /></div><span className="section-eyebrow">A fresh perspective</span><h1>Less noise.<br /><span>More room to think.</span></h1><p>Tell Forma what’s on your mind.<br />Let’s give your day a little shape.</p><Composer {...composerProps} large /><div className="welcome-chips"><button disabled={busy} onClick={() => runPreview('Arrange my day')}><Sun size={15} /> Bring my day together</button><button disabled={busy} onClick={() => runPreview('Focus on my inbox')}><Tray size={15} /> Find my focus</button><button disabled={busy} onClick={() => runPreview('Plan my trip to Lyon')}>Plan a little getaway <ArrowUp size={14} /></button></div>{chat.length > 1 && <div className="focus-answer" aria-live="polite"><FormaMark small /><p>{chat[chat.length - 1].text}</p></div>}<div className="embedded-preview-label"><span><SquaresFour size={16} /> Your space, taking shape</span><button onClick={() => setPage('saved')}>Saved spaces <ArrowRight size={14} /></button></div></section>}
            {variant === 'canvas' && <div className="canvas-toolbar"><div className="canvas-tabs"><button className="active" onClick={() => setPage('today')}><SquaresFour size={16} /> My daily space</button><button onClick={() => setPage('saved')}><BookmarkSimple size={16} /> Saved spaces {saved.length > 0 && <span>{saved.length}</span>}</button></div><button className="secondary-button" aria-expanded={canvasChat} onClick={() => setCanvasChat(value => !value)}><SidebarSimple size={16} />{canvasChat ? 'Hide conversation' : 'Shape this space'}</button></div>}
            {blank ? <section className="blank-workspace"><FormaMark /><h1>Start with a calmer day.</h1><p>A blank page, a little possibility.<br />Choose a sample or tell Forma what you need.</p><button className="primary-button" disabled={busy} onClick={() => runPreview('Arrange my day')}>{busy ? 'Arranging sample…' : 'Use sample day'}<ArrowRight size={16} /></button></section> : <WorkspaceView {...viewProps} minimalHero={variant === 'focus'} />}
            {variant === 'canvas' && !canvasChat && <div className="canvas-inline-composer"><div><Sparkle size={19} /><span>A workspace isn’t one-size-fits-all.<small>Tell us what would make this one yours.</small></span></div><Composer {...composerProps} large /></div>}
          </main>
          {variant === 'canvas' && canvasChat && <ChatPanel chat={chat} composer={composerProps} onExample={value => runPreview(value)} busy={busy} close={() => setCanvasChat(false)} />}
        </div>}
      </div>
    </div>
    <div className="sr-only" role="status" aria-live="polite">{busy ? 'Arranging scripted workspace.' : toast}</div>
    <AnimatePresence>{toast && <motion.div className="toast" key={toast} initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}><CheckCircle size={18} /><span>{toast}</span><IconButton label="Dismiss notification" onClick={() => setToast('')}><X size={14} /></IconButton></motion.div>}</AnimatePresence>
    {import.meta.env.DEV && <div className="variant-switcher" role="group" aria-label="Prototype design variants"><IconButton label="Previous design variant" onClick={() => changeVariant(VARIANTS[(VARIANTS.indexOf(variant) + 2) % 3])}><ArrowLeft size={15} /></IconButton><button className="variant-label" onClick={() => notify(`${variant} · revision ${workspace.revision} · ${saved.length} saved · scripted mode · session only`)} title="Show demo state"><span>{VARIANTS.indexOf(variant) + 1}<span className="variant-total"> / 3</span></span><span className="variant-divider" /><strong>{variant.charAt(0).toUpperCase() + variant.slice(1)}</strong></button><IconButton label="Next design variant" onClick={() => changeVariant(VARIANTS[(VARIANTS.indexOf(variant) + 1) % 3])}><ArrowRight size={15} /></IconButton></div>}
    {dialog === 'palette' && <Modal title="A shortcut to anywhere." onClose={() => setDialog(null)} className="command-palette"><div className="palette-search"><MagnifyingGlass size={21} /><input autoFocus aria-label="Search commands" value={paletteQuery} maxLength={100} onChange={event => setPaletteQuery(event.target.value)} placeholder="Where would you like to go?" /><kbd>esc</kbd></div><div className="palette-actions">{paletteActions.map((action, index) => <div key={action.label}>{(index === 0 || paletteActions[index - 1].group !== action.group) && <p className="palette-group">{action.group}</p>}<button disabled={action.disabled} onClick={() => { setDialog(null); action.action(); }}>{action.icon}<span>{action.label}</span><ArrowRight size={14} /></button></div>)}{!paletteActions.length && <p className="palette-empty">No commands match “{paletteQuery}”.</p>}</div><footer className="palette-footer"><span>Tab to explore · Enter to choose</span><span>Only this demo</span></footer></Modal>}
    {dialog === 'connections' && <Modal title="A little context. No connections." onClose={() => setDialog(null)} className="connection-sheet"><div className="connection-intro"><span className="connection-illustration"><EnvelopeSimple size={26} /><Plus size={15} /><CalendarBlank size={26} /></span><h3>Possibility, without the permissions.</h3><p>This preview uses invented messages, people, and calendar events. No account is connected, and no model is running.</p></div><div className="connection-row"><EnvelopeSimple size={21} /><div><strong>Messages</strong><span>Three sample conversations</span></div><span className="not-connected">Not connected</span></div><div className="connection-row"><CalendarBlank size={21} /><div><strong>Calendar</strong><span>A thoughtfully arranged sample day</span></div><span className="not-connected">Not connected</span></div><div className="connection-note"><Info size={18} /><p>Connecting accounts isn’t available in this visual demo. Your prompts and notes stay in this window and reset on reload. No credentials, files, or personal data are accessed.</p></div><button className="primary-button full-width" onClick={() => setDialog(null)}>Back to possibilities <ArrowRight size={16} /></button></Modal>}
    {dialog === 'versions' && <Modal title="A few ways your day could look." onClose={() => setDialog(null)} className="versions-sheet"><p className="modal-description">Saved snapshots live only in this session. Restore any version without removing the others.</p>{saved.length ? <div className="version-list">{[...saved].reverse().map(snapshot => <div className="version-row" key={snapshot.id}><span className="version-icon"><SquaresFour size={20} /></span><div><strong>{snapshot.name}</strong><span>Version {snapshot.workspace.revision} · {snapshot.workspace.quiet ? 'Quiet' : 'Comfortable'} layout</span></div><button className="secondary-button" onClick={() => requestRestore(snapshot)}>Restore <ArrowUUpLeft size={14} /></button></div>)}</div> : <div className="versions-empty"><BookmarkSimple size={30} /><h3>Your first save starts here.</h3><p>Save your current arrangement, make a change, then return to it anytime during this session.</p></div>}<button className="primary-button full-width" disabled={busy || previewError || blank || savedCurrent} onClick={() => { saveWorkspace(); }}>Save current version to memory <Plus size={16} /></button><p className="session-footnote">Session only · resets when this window reloads</p></Modal>}
    {dialog === 'restore' && restoreTarget && <Modal title="Return to this arrangement?" onClose={() => setDialog(null)} className="restore-sheet"><p className="modal-description">Your unsaved draft will be replaced by <strong>{restoreTarget.name}, version {restoreTarget.workspace.revision}</strong>. Your saved versions will stay in this session.</p><div className="dialog-buttons"><button className="secondary-button" onClick={() => setDialog('versions')}>Cancel</button><button className="primary-button" onClick={() => restore(restoreTarget)}>Discard draft and restore</button></div></Modal>}
    {dialog === 'inspector' && selected && <Modal title={selected.type === 'message' ? 'A closer look' : 'Room to prepare'} onClose={() => setDialog(null)} className="inspector-sheet"><InspectorContent key={selected.id} item={selected} workspace={workspace} onNote={note => { editWorkspace(current => ({ ...current, notes: { ...current.notes, [selected.id]: note } })); notify('Note kept in this demo · session only'); }} onDone={() => { editWorkspace(current => ({ ...current, done: current.done.includes(selected.id) ? current.done.filter(id => id !== selected.id) : [...current.done, selected.id] })); notify('Updated only in this demo · no external changes'); }} /></Modal>}
    {dialog === 'shortcuts' && <Modal title="Keep your hands in the flow." onClose={() => setDialog(null)} className="shortcuts-sheet"><p className="modal-description">A few small shortcuts. Your usual browser and system shortcuts are left alone.</p><dl className="shortcut-list"><div><dt>Open command menu</dt><dd><kbd>⌘ / Ctrl</kbd><kbd>K</kbd></dd></div><div><dt>Arrange a scripted preview</dt><dd><kbd>Enter</kbd></dd></div><div><dt>New line in your prompt</dt><dd><kbd>Shift</kbd><kbd>Enter</kbd></dd></div><div><dt>Keep a focused note</dt><dd><kbd>⌘ / Ctrl</kbd><kbd>Enter</kbd></dd></div><div><dt>Focus message search</dt><dd><kbd>/</kbd></dd></div><div><dt>Close a dialog</dt><dd><kbd>Esc</kbd></dd></div>{import.meta.env.DEV && <div><dt>Switch design on page background</dt><dd><kbd>←</kbd><kbd>→</kbd></dd></div>}</dl><p className="session-footnote">Arrow switching never interrupts interactive controls or text editing.</p></Modal>}
  </MotionConfig>;
}
