import { useEffect, useRef, useState, useSyncExternalStore, type FormEvent } from 'react';
import {
  ArrowDown, ArrowRight, ArrowUp, ChatCircle, Check, DotsThree,
  EnvelopeSimple, GearSix, MagnifyingGlass, PencilSimple, Plus, SidebarSimple,
  Sparkle, Square, Trash, X, CalendarBlank, Compass, SquaresFour, Clock, Browser as BrowserIcon,
} from '@phosphor-icons/react';
import { motion, useReducedMotion } from 'motion/react';
import { createBridge, friendlyError } from './bridge';
import { AppStore, type AppSnapshot } from './store';
import { AmbientMedia, IconButton, InlineError, Logo, Modal, SafeImage } from './components';
import { Onboarding, Settings } from './Settings';
import { ModelSelector } from './ModelSelector';
import { AssistantMessageContent } from './message';
import { BrowserView } from './BrowserView';
import { InterfacesView } from './InterfacesView';
import { SchedulesView } from './SchedulesView';
import { hermesCanAcceptMessage, ManagedHermesStatus, RuntimeActivity } from './RuntimeActivity';
import { globalShortcutBlocked } from './library-interactions';
import './runtime-ui.css';
import { useOwnedBrowser } from './owned-browser';
import type { ChatMessage, WorkspaceSummary } from './contracts';
import { media } from './media';
import './app.css';

const examples = [
  { title: 'Clear your inbox', detail: 'Make space for what matters', icon: EnvelopeSimple, theme: 'inbox',
    prompt: 'Help me clear my inbox. Ask me to paste the emails or a list of messages I want to sort, then help me prioritize them and draft any replies.' },
  { title: 'Plan your week', detail: 'A little structure, a lighter mind', icon: CalendarBlank, theme: 'week',
    prompt: 'Help me plan my week. Ask me about my commitments, priorities, and available time, then help me build a realistic plan with room to breathe.' },
  { title: 'Organize a trip', detail: 'Turn somewhere into a plan', icon: Compass, theme: 'trip',
    prompt: 'Help me organize a trip. Ask me where I’m thinking of going, my dates, budget, and preferences, then help me put together a thoughtful itinerary.' },
];

function quietDate(value: string) {
  const date = new Date(value);
  const days = Math.floor((Date.now() - date.getTime()) / 86_400_000);
  if (days <= 0) return 'Today';
  if (days === 1) return 'Yesterday';
  return date.toLocaleDateString(undefined, { month: 'short', day: 'numeric', ...(date.getFullYear() !== new Date().getFullYear() ? { year: 'numeric' as const } : {}) });
}

function EmptyChat({ name, motionEnabled, onExample }: { name: string; motionEnabled: boolean; onExample: (prompt: string) => void }) {
  const reduced = useReducedMotion();
  return <motion.section className="empty-chat" initial={{ opacity: 0, y: reduced ? 0 : 8 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.3 }}>
    <div className="empty-intro"><div><span className="eyebrow">A LITTLE SPACE, JUST FOR YOU</span><h1>{name ? `Hi ${name}.` : 'Make room for'}<br />{name ? 'What’s on your mind?' : 'what’s on your mind.'}</h1><p>Start with a thought. We’ll take it from there.</p></div><AmbientMedia enabled={motionEnabled} className="empty-ambient" /></div>
    <div className="discovery-heading"><span>A good place to start</span><span>Or make it your own <ArrowDown size={13} /></span></div>
    <div className="example-grid">{examples.map(example => <button key={example.title} type="button" className={`example-card ${example.theme}`} onClick={() => onExample(example.prompt)}>
      <div className={`example-art ${example.theme}`}>
        <SafeImage src={example.theme === 'trip' ? media.coast : media.cozy} />
        <span className="example-icon"><example.icon size={30} weight="light" /></span>
        <span className="art-line art-line-one" /><span className="art-line art-line-two" />
      </div>
      <div className="example-copy"><span>{example.title}</span><ArrowRight size={16} /><small>{example.detail}</small></div>
    </button>)}</div>
  </motion.section>;
}

function Message({ message, onRetry, pending }: { message: ChatMessage; onRetry: () => void; pending: boolean }) {
  if (message.role === 'user') return <article className="message user-message" aria-label="Your message"><div>{message.content}</div></article>;
  return <article className="message assistant-message" aria-label="Forma reply">
    <Logo compact />
    <div className="assistant-body">
      {message.status === 'complete' ? <AssistantMessageContent message={message} />
        : message.status === 'pending' && pending ? <div className="reply-pending" role="status"><span className="thinking-dots"><i /><i /><i /></span><span>Thinking it through…</span></div>
          : <div className="reply-interrupted"><span>{message.status === 'cancelled' ? 'Reply stopped.' : 'This reply didn’t finish.'}</span><button type="button" className="text-button" onClick={onRetry}>Edit & retry <ArrowRight size={14} /></button></div>}
    </div>
  </article>;
}

function WorkspaceMenu({ onRename, onDelete }: { onRename: () => void; onDelete: () => void }) {
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => { if (!root.current?.contains(event.target as Node)) setOpen(false); };
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') { event.stopPropagation(); setOpen(false); root.current?.querySelector('button')?.focus(); } };
    document.addEventListener('pointerdown', outside); document.addEventListener('keydown', escape, true);
    return () => { document.removeEventListener('pointerdown', outside); document.removeEventListener('keydown', escape, true); };
  }, [open]);
  return <div ref={root} className="workspace-menu">
    <button className="icon-button" type="button" aria-label="Workspace options" title="Workspace options" aria-expanded={open} onClick={() => setOpen(!open)}><DotsThree size={23} /></button>
    {open && <div className="workspace-dropdown"><button type="button" onClick={() => { setOpen(false); onRename(); }}><PencilSimple size={17} />Rename workspace</button><button type="button" className="danger-text" onClick={() => { setOpen(false); onDelete(); }}><Trash size={17} />Delete workspace</button></div>}
  </div>;
}

function WorkspaceDialog({ mode, workspace, store, onClose }: { mode: 'rename' | 'delete'; workspace: WorkspaceSummary; store: AppStore; onClose: () => void }) {
  const [title, setTitle] = useState(workspace.title);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const submit = async (event: FormEvent) => {
    event.preventDefault(); if (busy) return;
    setBusy(true); setError(null);
    try {
      if (mode === 'delete') await store.deleteWorkspace(workspace.id);
      else await store.renameWorkspace(workspace.id, title.trim());
      onClose();
    } catch (failure) { setError(friendlyError(failure)); }
    finally { setBusy(false); }
  };
  return <Modal title={mode === 'delete' ? 'Delete this workspace?' : 'Rename workspace'} onClose={onClose} dismissible={!busy}>
    <form onSubmit={event => void submit(event)}>
      {mode === 'delete' ? <><div className="delete-symbol"><Trash size={25} /></div><p className="dialog-description">“{workspace.title}” and its messages and draft will be removed from this device. Shared interfaces remain in the library; associated schedules are stopped by the runtime. This can’t be undone.</p></>
        : <label className="rename-field">Workspace name<input autoFocus value={title} maxLength={200} onChange={event => setTitle(event.target.value)} required disabled={busy} /></label>}
      {error && <InlineError>{error}</InlineError>}
      <div className="modal-actions"><button type="button" className="button secondary" disabled={busy} onClick={onClose}>Keep {mode === 'delete' ? 'workspace' : 'name'}</button><button type="submit" className={`button ${mode === 'delete' ? 'danger' : 'primary'}`} disabled={busy || (mode === 'rename' && !title.trim())}>{busy ? 'One moment…' : mode === 'delete' ? 'Delete workspace' : 'Rename'}</button></div>
    </form>
  </Modal>;
}

function CommandPalette({ state, store, onClose, onSettings, onChat, onBrowser, onInterfaces, onSchedules }: { state: AppSnapshot; store: AppStore; onClose: () => void; onSettings: () => void; onChat: () => void; onBrowser: () => void; onInterfaces: () => void; onSchedules: () => void }) {
  const [query, setQuery] = useState('');
  const [index, setIndex] = useState(0);
  const actions = [
    { id: 'new', label: 'New workspace', icon: Plus, shortcut: '⇧⌘N', run: () => { onChat(); void store.newWorkspace().catch(() => {}); } },
    { id: 'browser', label: 'Your browser', icon: BrowserIcon, shortcut: '', run: onBrowser },
    { id: 'interfaces', label: 'Interfaces', icon: SquaresFour, shortcut: '', run: onInterfaces },
    { id: 'schedules', label: 'Schedules', icon: Clock, shortcut: '', run: onSchedules },
    { id: 'settings', label: 'Settings', icon: GearSix, shortcut: '', run: onSettings },
    ...state.workspaces.map(workspace => ({ id: workspace.id, label: workspace.title, icon: ChatCircle, shortcut: '', run: () => { onChat(); void store.selectWorkspace(workspace.id).catch(() => {}); } })),
  ].filter(action => action.label.toLocaleLowerCase().includes(query.toLocaleLowerCase()));
  const choose = (choice: number) => { const action = actions[choice]; if (action) { onClose(); action.run(); } };
  return <Modal title="Find your space" onClose={onClose} className="command-modal">
    <div className="command-search"><MagnifyingGlass size={20} /><input autoFocus value={query} placeholder="Find a workspace or action…" aria-label="Search workspaces and actions" role="combobox" aria-expanded="true" aria-controls="command-results" aria-activedescendant={actions[index] ? `command-${actions[index].id}` : undefined}
      onChange={event => { setQuery(event.target.value); setIndex(0); }} onKeyDown={event => {
        if (event.key === 'ArrowDown') { event.preventDefault(); setIndex(value => Math.min(actions.length - 1, value + 1)); }
        if (event.key === 'ArrowUp') { event.preventDefault(); setIndex(value => Math.max(0, value - 1)); }
        if (event.key === 'Enter') { event.preventDefault(); choose(index); }
      }} /></div>
    <div className="command-results" id="command-results" role="listbox" aria-label="Workspaces and actions">{actions.map((action, position) => <button id={`command-${action.id}`} type="button" role="option" aria-selected={position === index} key={action.id} onMouseMove={() => setIndex(position)} onClick={() => choose(position)}><action.icon size={18} /><span>{action.label}</span>{action.shortcut && <kbd>{action.shortcut}</kbd>}</button>)}</div>
    {!actions.length && <p className="search-empty">No matching workspaces. Try a shorter name, or clear your search.</p>}
    <div className="palette-footer"><span><kbd>↑</kbd><kbd>↓</kbd> to move</span><span><kbd>↵</kbd> to open</span><span><kbd>esc</kbd> to close</span></div>
  </Modal>;
}

function useLifecycle(store: AppStore) {
  useEffect(() => {
    void store.initialise();
    let dispose: (() => void) | undefined;
    let disposed = false;
    let closing = false;
    if (store.bridge.native) {
      void import('@tauri-apps/api/window').then(async ({ getCurrentWindow }) => {
        const window = getCurrentWindow();
        const unlisten = await window.onCloseRequested(async event => {
          event.preventDefault();
          if (closing) return;
          closing = true;
          try {
            await store.prepareClose();
            if (!store.bridge.finishWindowClose) throw new Error('Native close unavailable');
            const result = await store.bridge.finishWindowClose();
            if (result === 'hidden') { store.resumeAfterClose(); closing = false; }
          }
          catch { store.closeFailed(); closing = false; }
        });
        if (disposed) unlisten(); else dispose = unlisten;
      }).catch(() => store.closeFailed());
    }
    const hide = () => { if (document.visibilityState === 'hidden') void store.flush().catch(() => {}); };
    const beforeUnload = (event: BeforeUnloadEvent) => {
      const state = store.getSnapshot();
      void store.flush().catch(() => {});
      if (state.dirty || state.saving) { event.preventDefault(); event.returnValue = ''; }
    };
    document.addEventListener('visibilitychange', hide);
    if (!store.bridge.native) window.addEventListener('beforeunload', beforeUnload);
    return () => { disposed = true; dispose?.(); document.removeEventListener('visibilitychange', hide); window.removeEventListener('beforeunload', beforeUnload); };
  }, [store]);
}

export function App() {
  const [store] = useState(() => new AppStore(createBridge()));
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot);
  const browser = useOwnedBrowser(store.bridge.native);
  const [view, setView] = useState<'chat' | 'browser' | 'interfaces' | 'schedules'>('chat');
  const [draftNotice, setDraftNotice] = useState('');
  const [interfaceFocus, setInterfaceFocus] = useState<string | undefined>();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [shortcutsOpen, setShortcutsOpen] = useState(false);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [workspaceDialog, setWorkspaceDialog] = useState<{ mode: 'rename' | 'delete'; workspace: WorkspaceSummary } | null>(null);
  const composer = useRef<HTMLTextAreaElement>(null);
  const scroll = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const [showJump, setShowJump] = useState(false);
  useLifecycle(store);
  useEffect(() => {
    if (!store.bridge.native || state.closing || state.runtime?.state !== 'starting') return;
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    const refresh = async () => {
      if (disposed || store.getSnapshot().closing || store.getSnapshot().runtime?.state !== 'starting') return;
      try { await store.refreshRuntime(); } catch { /* Store owns the safe status error. */ }
      if (!disposed && !store.getSnapshot().closing && store.getSnapshot().runtime?.state === 'starting') timer = setTimeout(() => void refresh(), 1000);
    };
    timer = setTimeout(() => void refresh(), 1000);
    return () => { disposed = true; clearTimeout(timer); };
  }, [store, state.runtime?.state, state.closing]);
  const focusComposer = () => { requestAnimationFrame(() => composer.current?.focus()); };
  const fillComposer = (prompt: string) => { store.editDraft(prompt); focusComposer(); };
  const send = () => {
    if (!state.draft.trim() || state.pending || state.providerBusy || state.selectingModel) return;
    if (store.bridge.native && state.runtimeWorkspace?.workspaceId !== state.activeId) { void store.refreshRuntime().catch(() => {}); return; }
    if (!hermesCanAcceptMessage(store.bridge.native, state.runtime, !!state.settings?.provider.verified, state.runtimeWorkspace?.modelId || '')) { setSettingsOpen(true); return; }
    follow.current = true;
    void store.send().catch(() => setSettingsOpen(true));
  };
  useEffect(() => {
    const input = composer.current;
    if (input) { input.style.height = 'auto'; input.style.height = `${Math.min(input.scrollHeight, 180)}px`; }
  }, [state.draft, state.ready, state.settings?.onboardingComplete]);
  useEffect(() => { follow.current = true; setShowJump(false); }, [state.activeId]);
  useEffect(() => { if (follow.current && scroll.current) scroll.current.scrollTop = scroll.current.scrollHeight; }, [state.workspace?.messages, state.pending, state.activeId, view]);
  useEffect(() => {
    const keydown = (event: KeyboardEvent) => {
      if (globalShortcutBlocked(event, document, settingsOpen || paletteOpen || shortcutsOpen || !!workspaceDialog)) return;
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') { event.preventDefault(); setPaletteOpen(true); }
      else if ((event.metaKey || event.ctrlKey) && event.shiftKey && event.key.toLowerCase() === 'n') { event.preventDefault(); setView('chat'); void store.newWorkspace().then(focusComposer).catch(() => {}); }
      else if (event.key === '/') { event.preventDefault(); if (view === 'browser') document.querySelector<HTMLInputElement>('[aria-label="Website address"]')?.focus(); else if (view === 'interfaces' || view === 'schedules') document.querySelector<HTMLInputElement>('.library-page:not([hidden]) [data-library-search]')?.focus(); else focusComposer(); }
      else if (event.key === '?') { event.preventDefault(); setShortcutsOpen(true); }
      else if (event.key === 'Escape') { if (view === 'chat' && state.pending) store.cancel(); setSidebarOpen(false); }
    };
    if (state.settings?.onboardingComplete) window.addEventListener('keydown', keydown);
    return () => window.removeEventListener('keydown', keydown);
  }, [state.pending, state.settings?.onboardingComplete, store, view, settingsOpen, paletteOpen, shortcutsOpen, workspaceDialog]);

  if (!state.ready || !state.settings) return <main className="startup-screen"><Logo /><div className="startup-content">{state.error ? <><h1>Your space is still here.</h1><InlineError onRetry={() => void store.initialise()}>{state.error}</InlineError></> : <><div className="skeleton skeleton-title" /><div className="skeleton skeleton-line" /><span className="sr-only" role="status">Opening your workspaces</span></>}</div></main>;
  const settings = state.settings;
  const settingsModal = settingsOpen && <Settings store={store} settings={settings} browser={browser} onClose={() => { setSettingsOpen(false); if (view === 'chat') focusComposer(); }} />;
  if (!settings.onboardingComplete) return <><Onboarding store={store} settings={settings} browser={browser} onSettings={() => setSettingsOpen(true)} />{settingsModal}</>;
  const workspace = state.workspace;
  const empty = !workspace?.messages.length;
  const needsReconcile = store.bridge.native && !state.pending && !!workspace?.messages.some(message => message.status === 'pending');
  return <div className={`app-shell ${sidebarOpen ? 'sidebar-is-open' : ''}`}>
    <a className="skip-link" href={view === 'browser' ? '#browser-title' : view === 'interfaces' ? '#interfaces-title' : view === 'schedules' ? '#schedules-title' : '#chat-composer'}>Skip to {view}</a>
    {sidebarOpen && <button type="button" className="sidebar-scrim" aria-label="Close workspace sidebar" onClick={() => setSidebarOpen(false)} />}
    <aside className="app-sidebar" aria-label="Workspaces">
      <div className="sidebar-brand"><Logo /><IconButton label="Find a workspace (⌘K)" onClick={() => setPaletteOpen(true)}><MagnifyingGlass size={18} /></IconButton></div>
      <button className="new-chat-button" type="button" title="New workspace (⌘⇧N)" disabled={state.navigating || state.closing} onClick={() => { setView('chat'); void store.newWorkspace().then(focusComposer).catch(() => {}); setSidebarOpen(false); }}><Plus size={19} /><span>New workspace</span><kbd>⇧⌘N</kbd></button>
      <button type="button" className={`browser-nav ${view === 'browser' ? 'selected' : ''}`} aria-current={view === 'browser' ? 'page' : undefined} disabled={state.closing} onClick={() => { setView('browser'); setSidebarOpen(false); }}><BrowserIcon size={19} /><span>Browser</span></button>
      <button type="button" className={`browser-nav library-nav ${view === 'interfaces' ? 'selected' : ''}`} aria-current={view === 'interfaces' ? 'page' : undefined} disabled={state.closing} onClick={() => { setView('interfaces'); setInterfaceFocus(undefined); setSidebarOpen(false); }}><SquaresFour size={19} /><span>Interfaces</span></button>
      <button type="button" className={`browser-nav library-nav ${view === 'schedules' ? 'selected' : ''}`} aria-current={view === 'schedules' ? 'page' : undefined} disabled={state.closing} onClick={() => { setView('schedules'); setSidebarOpen(false); }}><Clock size={19} /><span>Schedules</span></button>
      <div className="history-heading"><span>Your workspaces</span><span>{state.workspaces.length}</span></div>
      <nav className="workspace-history" aria-label="Past workspaces">{state.workspaces.map(item => <button type="button" key={item.id} className={`history-item ${view === 'chat' && state.activeId === item.id ? 'selected' : ''}`} aria-current={view === 'chat' && state.activeId === item.id ? 'page' : undefined}
        disabled={state.navigating || state.closing} onClick={() => { setView('chat'); void store.selectWorkspace(item.id).then(focusComposer).catch(() => {}); setSidebarOpen(false); }} title={item.title}>
        <ChatCircle size={17} /><span><span className="history-title">{item.title}</span><time dateTime={item.updatedAt} title={new Date(item.updatedAt).toLocaleString()}>{quietDate(item.updatedAt)}</time></span>
      </button>)}</nav>
      <div className="sidebar-bottom"><button type="button" className="profile-button" onClick={() => setSettingsOpen(true)}><span className="profile-avatar">{settings.displayName ? Array.from(settings.displayName)[0].toLocaleUpperCase() : <Sparkle size={17} />}</span><span>{settings.displayName || 'Your space'}<small>Settings & connections</small></span><GearSix size={17} /></button></div>
    </aside>
    <main className="chat-canvas" id="main-content">
      <header className="chat-topbar" data-tauri-drag-region>
        <div className="topbar-title"><IconButton className="mobile-sidebar-toggle" label="Show workspaces" onClick={() => setSidebarOpen(true)}><SidebarSimple size={20} /></IconButton><span>{view === 'browser' ? 'Browser' : view === 'interfaces' ? 'Interfaces' : view === 'schedules' ? 'Schedules' : workspace?.title || 'Your space'}</span></div>
        <div className="topbar-actions">{state.navigating && <span className="quiet-status" role="status">Opening…</span>}{view === 'chat' && workspace && <WorkspaceMenu onRename={() => setWorkspaceDialog({ mode: 'rename', workspace })} onDelete={() => setWorkspaceDialog({ mode: 'delete', workspace })} />}</div>
      </header>
      <InterfacesView key={`interfaces-${state.runtime?.generation}-${state.activeId}`} active={view === 'interfaces'} native={store.bridge.native} status={state.runtime} workspaceId={state.activeId} focusId={interfaceFocus} onSettings={() => setSettingsOpen(true)} onRevise={prompt => { setView('chat'); const draft = store.getSnapshot().draft; const combined = draft.trim() ? `${draft}\n\n${prompt}` : prompt; if (combined.length > 32_000) { setDraftNotice('Your existing draft is too long to append an interface request. It has been preserved; shorten or send it first.'); focusComposer(); } else { setDraftNotice(draft.trim() ? 'Interface revision request appended to your existing draft. Review it and Send when ready.' : 'Interface request added to your draft. Review it and Send when ready.'); fillComposer(combined); } }} />
      <SchedulesView key={`schedules-${state.runtime?.generation}-${state.activeId}`} active={view === 'schedules'} native={store.bridge.native} status={state.runtime} workspaceId={state.activeId} modelId={state.runtimeWorkspace?.workspaceId === state.activeId ? state.runtimeWorkspace?.modelId || '' : ''} onSettings={() => setSettingsOpen(true)} />
      {view === 'browser' ? <BrowserView browser={browser} /> : view === 'chat' ? <>
      {state.error && <div className="global-error"><InlineError>{state.error}</InlineError><IconButton label="Dismiss notice" onClick={store.clearError}><X size={16} /></IconButton></div>}
      <div className={`chat-scroll ${empty ? 'is-empty' : ''}`} ref={scroll} onScroll={() => { const element = scroll.current; if (element) { follow.current = element.scrollHeight - element.scrollTop - element.clientHeight < 100; setShowJump(!follow.current); } }}>
        <div className="chat-width">{empty ? <EmptyChat name={settings.displayName} motionEnabled={settings.ambientMotion} onExample={fillComposer} />
          : <div className="conversation" aria-label="Chat messages">{workspace.messages.map((message, index) => <Message key={message.id} message={message} pending={state.pending}
            onRetry={() => { const prompt = workspace.messages.slice(0, index).reverse().find(item => item.role === 'user')?.content; if (prompt) fillComposer(prompt); }} />)}
            {state.starting && <div className="starting-message" role="status">Getting your message ready…</div>}
          </div>}
          <RuntimeActivity progress={state.runtimeProgress} workspaceId={state.activeId} onInterfaces={id => { setInterfaceFocus(id); setView('interfaces'); }} />
        </div>
      </div>
      <div className="composer-area"><div className="chat-width">
        {showJump && !empty && <button className="jump-button" type="button" onClick={() => { if (scroll.current) scroll.current.scrollTop = scroll.current.scrollHeight; follow.current = true; setShowJump(false); }}><ArrowDown size={17} />Latest message</button>}
        {draftNotice && <p className="field-note" role="status">{draftNotice}</p>}
        {needsReconcile && <div className="library-notice"><p>A previous Hermes run has an unconfirmed outcome. Reconcile its durable status before sending another message; no run will be resubmitted.</p><button type="button" className="button secondary" disabled={state.navigating || state.closing} onClick={() => { void store.resumeRuntimeReply().catch(() => {}); }}>Reconcile previous run</button></div>}
        {state.replyError && <InlineError>{state.replyError}</InlineError>}
        {state.draftError && <InlineError onRetry={() => { void store.retryDraft().catch(() => {}); }}>{state.draftError}</InlineError>}
        <ManagedHermesStatus state={state} store={store} onSettings={() => setSettingsOpen(true)} />
        <form className={`composer ${state.draftError ? 'has-error' : ''}`} onSubmit={event => { event.preventDefault(); send(); }}>
          <label className="sr-only" htmlFor="chat-composer">Message Forma</label>
          <textarea id="chat-composer" ref={composer} rows={2} value={state.draft} maxLength={32_000} placeholder="What would you like a hand with?" disabled={state.starting || state.navigating || state.closing || !workspace}
            onChange={event => store.editDraft(event.target.value)} onKeyDown={event => {
              if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); send(); }
            }} />
          <div className="composer-toolbar"><ModelSelector state={state} store={store} onSettings={() => setSettingsOpen(true)} />
            <div className="composer-right"><span className="composer-shortcut">{state.pending ? 'Take your time' : '↵ to send'}</span>{state.pending ? <button className="send-button stop-button" type="button" aria-label="Stop reply (Esc)" title="Stop reply (Esc)" onClick={store.cancel}><Square size={15} weight="fill" /></button>
              : <button type="submit" className="send-button" aria-label="Send message" title="Send message (Enter)" disabled={(store.bridge.native && state.runtimeWorkspace?.workspaceId !== state.activeId) || needsReconcile || !state.draft.trim() || state.navigating || state.closing || state.providerBusy || state.selectingModel}><ArrowUp size={20} weight="bold" /></button>}</div>
          </div>
        </form>
        <div className="composer-footer"><span role="status">{state.closing ? 'Keeping your work safe before closing…' : state.saving || state.dirty ? 'Remembering your draft…' : <><Check size={12} />Remembered on this device</>}</span><button type="button" onClick={() => setShortcutsOpen(true)} title="Keyboard shortcuts (?)">Keyboard shortcuts <kbd>?</kbd></button></div>
      </div></div>
      </> : null}
    </main>
    {settingsModal}
    {paletteOpen && <CommandPalette state={state} store={store} onClose={() => setPaletteOpen(false)} onSettings={() => setSettingsOpen(true)} onChat={() => setView('chat')} onBrowser={() => setView('browser')} onInterfaces={() => setView('interfaces')} onSchedules={() => setView('schedules')} />}
    {workspaceDialog && <WorkspaceDialog mode={workspaceDialog.mode} workspace={workspaceDialog.workspace} store={store} onClose={() => setWorkspaceDialog(null)} />}
    {shortcutsOpen && <Modal title="A few handy shortcuts" onClose={() => setShortcutsOpen(false)} className="shortcuts-modal"><dl>{[['Find a workspace', '⌘ / Ctrl K'], ['New workspace', '⌘ / Ctrl ⇧ N'], ['Focus your message', '/'], ['Send a message', 'Enter'], ['Add a new line', 'Shift Enter'], ['Close a dialog / stop a reply', 'Esc']].map(([label, key]) => <div key={label}><dt>{label}</dt><dd><kbd>{key}</kbd></dd></div>)}</dl></Modal>}
  </div>;
}
