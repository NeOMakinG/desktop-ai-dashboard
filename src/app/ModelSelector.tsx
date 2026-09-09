import { useEffect, useId, useRef, useState } from 'react';
import { CaretDown, Check, MagnifyingGlass } from '@phosphor-icons/react';
import type { AppSnapshot, AppStore } from './store';
import { InlineError, Modal } from './components';

/** Labels are presentation only; selection always uses the exact discovered ID. */
export function modelLabel(id: string): string {
  const claude = /^claude-(opus|sonnet|haiku)-(\d{1,3})(?:-(\d{1,3}))?(?:-|$)/.exec(id);
  if (!claude) return id || 'Choose model';
  return `${claude[1][0].toUpperCase()}${claude[1].slice(1)} ${claude[2]}${claude[3] ? `.${claude[3]}` : ''}`;
}
export function filterModels(models: string[], query: string): string[] {
  const needle = query.trim().toLocaleLowerCase();
  return models.filter(id => `${modelLabel(id)} ${id}`.toLocaleLowerCase().includes(needle));
}

export function ModelSelector({ state, store, onSettings }: { state: AppSnapshot; store: AppStore; onSettings: () => void }) {
  const [open, setOpen] = useState(false);
  const current = state.settings?.provider;
  const blocked = state.anyReplyPending || state.providerBusy || state.selectingModel || state.closing;
  const show = () => {
    setOpen(true);
    if (store.bridge.native && current?.baseUrl && state.modelsStatus === 'idle') void store.refreshModels().catch(() => {});
  };
  return <>
    <button type="button" className="model-chip" aria-haspopup="dialog" aria-expanded={open}
      aria-label={`Choose model, current: ${current?.model ? modelLabel(current.model) : 'not selected'}${current?.model && !current.verified ? ', needs a check' : ''}`}
      title={state.anyReplyPending ? 'Finish or stop active replies to change models' : current?.model || 'Choose a model'} disabled={blocked} onClick={show}>
      <span>{state.selectingModel ? 'Checking model…' : modelLabel(current?.model ?? '')}</span>
      {current?.model && !current.verified && <span className="model-chip-note">Unchecked</span>}<CaretDown size={12} />
    </button>
    {open && <ModelPicker state={state} store={store} onClose={() => setOpen(false)} onSettings={() => { setOpen(false); onSettings(); }} />}
  </>;
}

export function ModelPicker({ state, store, onClose, onSettings }: { state: AppSnapshot; store: AppStore; onClose: () => void; onSettings: () => void }) {
  const [query, setQuery] = useState('');
  const [index, setIndex] = useState(0);
  const listId = useId();
  const list = useRef<HTMLDivElement>(null);
  const models = filterModels(state.models, query);
  const activeIndex = Math.max(0, Math.min(index, models.length - 1));
  const current = state.settings?.provider;
  const busy = state.selectingModel || state.providerBusy || state.closing;
  const blocked = busy || state.anyReplyPending;
  useEffect(() => { list.current?.querySelector('[data-active="true"]')?.scrollIntoView({ block: 'nearest' }); }, [activeIndex, query, state.models]);
  const choose = async (id: string) => {
    if (blocked) return;
    try { await store.selectModel(id); onClose(); } catch { /* Store retains recovery choices and a safe error. */ }
  };
  const refresh = () => { void store.refreshModels().catch(() => {}); };
  return <Modal title="Choose a model" onClose={onClose} className="model-modal" dismissible={!busy}>
    <p className="field-note model-intro">Used for future replies in all workspaces.</p>
    {!store.bridge.native ? <p className="search-empty">Models are available in the desktop app. This preview keeps your drafts, without connecting to AI.</p>
      : !current?.baseUrl ? <p className="search-empty">Set up your AI connection to see its available models.</p>
        : <>
          <div className="command-search"><MagnifyingGlass size={19} /><input autoFocus value={query} placeholder="Search models…" aria-label="Search models"
            role="combobox" aria-autocomplete="list" aria-expanded="true" aria-controls={listId}
            aria-activedescendant={models.length ? `${listId}-${activeIndex}` : undefined}
            onChange={event => { setQuery(event.target.value); setIndex(0); }} onKeyDown={event => {
              if (event.nativeEvent.isComposing) return;
              if (event.key === 'ArrowDown') { event.preventDefault(); setIndex(Math.min(models.length - 1, activeIndex + 1)); }
              if (event.key === 'ArrowUp') { event.preventDefault(); setIndex(Math.max(0, activeIndex - 1)); }
              if (event.key === 'Enter') { event.preventDefault(); if (models[activeIndex]) void choose(models[activeIndex]); }
            }} /></div>
          <div className="model-list-heading"><span role="status">{state.modelsStatus === 'loading' ? 'Refreshing models…' : `${state.models.length} available`}</span>
            <button type="button" className="text-button muted" disabled={busy || state.modelsStatus === 'loading'} onClick={refresh}>Refresh</button></div>
          {state.modelsError && <InlineError>{state.modelsError} {state.models.length > 0 && 'Previously discovered models are still listed.'}</InlineError>}
          {state.modelSelectionError && <InlineError>{state.modelSelectionError}</InlineError>}
          {state.anyReplyPending && <p className="field-note">Finish or stop active replies before changing models.</p>}
          {current.model && state.modelsStatus === 'ready' && !state.models.includes(current.model) && <p className="field-note model-notice">Your saved model is unavailable. Choose another model to reconnect.</p>}
          <div className="model-results" id={listId} ref={list} role="listbox" aria-label="Available models" aria-busy={state.modelsStatus === 'loading'}>
            {models.map((id, position) => <button type="button" role="option" id={`${listId}-${position}`} key={id} tabIndex={-1}
              aria-selected={id === current.model} data-active={position === activeIndex} disabled={blocked}
              onMouseMove={() => setIndex(position)} onClick={() => void choose(id)}>
              <span><span>{modelLabel(id)}</span><small>{id}</small></span>
              {id === current.model && <span className="model-current">{current.verified ? <><Check size={14} />Current</> : 'Needs a check'}</span>}
            </button>)}
          </div>
          {!models.length && state.modelsStatus !== 'loading' && <p className="search-empty">{state.models.length ? 'No matching models. Try a shorter search.' : state.modelsStatus === 'ready' ? 'No models returned. Refresh or check your connection settings.' : 'Refresh to discover models from your connection.'}</p>}
          {state.selectingModel && <p className="field-note" role="status">Saving and checking your model…</p>}
          <div className="palette-footer"><span><kbd>↑</kbd><kbd>↓</kbd> to move</span><span><kbd>↵</kbd> to choose</span><span><kbd>esc</kbd> to close</span></div>
        </>}
    <div className="model-settings"><button type="button" className="text-button muted" disabled={busy} onClick={onSettings}>Connection settings</button></div>
  </Modal>;
}
