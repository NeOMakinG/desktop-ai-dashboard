import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { ArrowRight, ArrowSquareOut, Check, CheckCircle, EnvelopeSimple, GearSix, GoogleLogo, LockSimple, Sparkle, X } from '@phosphor-icons/react';
import { AccountGrantStatus, RuntimeSettings } from './RuntimeSettings';
import type { AppSettings } from './contracts';
import type { AppStore } from './store';
import { friendlyError } from './bridge';
import { AmbientMedia, InlineError, Logo, Modal } from './components';
import { BrowserConnections } from './BrowserView';
import type { OwnedBrowser } from './owned-browser';
import type { ConnectorStatus } from './browser-contracts';
import { GOOGLE_DEFAULT_SCOPES, useConnectors, type ConnectorsController } from './connectors';

function GoogleAccountRow({ item, controller }: { item: ConnectorStatus; controller: ConnectorsController }) {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!open) return;
    const handler = (event: MouseEvent) => {
      if (anchor.current && !anchor.current.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, [open]);
  const scopeSummary = item.scopes.length === 1 ? '1 scope' : `${item.scopes.length} scopes`;
  return <div className="account-row connector-row">
    <span className="account-icon"><GoogleLogo size={20} /></span>
    <span>Google account<span className="row-detail">Signed in · {scopeSummary}</span></span>
    <span className="connector-badge" aria-label="Connected"><CheckCircle size={13} weight="fill" />Connected</span>
    <div className="connector-menu-anchor" ref={anchor}>
      <button type="button" className="text-button muted" aria-haspopup="menu" aria-expanded={open} disabled={controller.busy}
        onClick={() => setOpen(value => !value)}>Manage</button>
      {open && <div className="connector-popover" role="menu">
        <button type="button" className="text-button" role="menuitem" disabled={controller.busy}
          onClick={() => { setOpen(false); void controller.disconnect(item.id); }}><X size={14} />Disconnect</button>
      </div>}
    </div>
  </div>;
}

export function Accounts({ browser }: { browser: OwnedBrowser }) {
  const connectors = useConnectors(browser.native);
  const googleAccounts = connectors.items.filter(item => item.provider === 'google');
  const capabilities = connectors.capabilities;
  const signIn = useRef<HTMLButtonElement | null>(null);
  const cancelHadFocus = useRef(false);
  useEffect(() => {
    if (!connectors.pending && !connectors.busy && cancelHadFocus.current && signIn.current && !signIn.current.disabled) {
      signIn.current.focus();
      cancelHadFocus.current = false;
    }
  }, [connectors.pending, connectors.busy, browser.busy]);
  const disabled = !browser.native || !browser.status.available || browser.busy || connectors.busy || !capabilities.googleAvailable;
  const detail = connectors.pending
    ? (connectors.attempt?.phase === 'exchanging' ? 'Finishing Google sign-in…' : 'Waiting for Google sign-in…')
    : !capabilities.googleAvailable
      ? (capabilities.disabledReason || 'Google OAuth client not configured yet.')
      : googleAccounts.length > 0
        ? 'Add another Google account'
        : 'Request Gmail metadata & Calendar read-only permissions';
  const startSignIn = async () => {
    const response = await connectors.startGoogle(capabilities.defaultScopes.length ? capabilities.defaultScopes : GOOGLE_DEFAULT_SCOPES);
    if (response && !await browser.open('custom', response.authorizeUrl)) {
      await connectors.cancel(response.attemptId);
    }
  };
  return <div className="browser-connections">
    <div className="account-row connector-row">
      <span className="account-icon"><GoogleLogo size={20} /></span>
      <span>Google<span className="row-detail" role="status" aria-live="polite">{detail}</span></span>
      <button ref={signIn} type="button" className="button secondary" disabled={disabled} onClick={() => { void startSignIn(); }}>Sign in <ArrowSquareOut size={14} /></button>
      {connectors.pending && <button type="button" className="text-button" disabled={connectors.cancelling}
        onFocus={() => { cancelHadFocus.current = true; }} onBlur={() => { cancelHadFocus.current = false; }}
        onClick={() => { void connectors.cancel(); }}>{connectors.cancelling ? 'Cancelling…' : 'Cancel'}</button>}
    </div>
    {googleAccounts.map(item => <GoogleAccountRow key={item.id} item={item} controller={connectors} />)}
    {connectors.cleanupIds.map(id => <div className="account-row connector-row" key={id}>
      <span className="account-icon"><GoogleLogo size={20} /></span>
      <span>Google credential cleanup<span className="row-detail">Disconnected locally. Credential cleanup is incomplete.</span></span>
      <button type="button" className="text-button" disabled={connectors.busy} onClick={() => { void connectors.disconnect(id); }}>Retry cleanup</button>
    </div>)}
    {connectors.notice && <p className="field-note" role="status">{connectors.notice}</p>}
    {connectors.error && <InlineError>{connectors.error}</InlineError>}
    <BrowserConnections browser={browser} />
  </div>;
}

function ProviderSetup({ store, settings, onBusyChange }: { store: AppStore; settings: AppSettings; onBusyChange?: (busy: boolean) => void }) {
  const provider = settings.provider;
  const [baseUrl, setBaseUrl] = useState(provider.baseUrl);
  const [model, setModel] = useState(provider.model);
  const [apiKey, setApiKey] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot);
  const models = state.models;
  const locked = busy || state.providerBusy || state.selectingModel || state.anyReplyPending || state.closing;
  const [checked, setChecked] = useState(false);
  const changed = baseUrl !== provider.baseUrl || model !== provider.model || !!apiKey;
  const markBusy = (value: boolean) => { setBusy(value); onBusyChange?.(value); };
  const submit = async () => {
    if (!store.bridge.native || locked) return;
    markBusy(true); setError(null); setChecked(false);
    try {
      if (changed || !provider.baseUrl) {
        const saved = await store.configureProvider({ label: 'Custom AI', baseUrl: baseUrl.trim(), model: model.trim(), ...(apiKey ? { apiKey } : {}) });
        setApiKey('');
        setBaseUrl(saved.baseUrl); setModel(saved.model);
      }
      const result = await store.checkProvider();
      setModel(result.provider.model); setBaseUrl(result.provider.baseUrl); setChecked(true);
    } catch (failure) { setError(friendlyError(failure)); }
    finally { setApiKey(''); markBusy(false); }
  };
  const removeKey = async () => {
    if (locked) return;
    markBusy(true); setError(null); setChecked(false); setApiKey('');
    try {
      const saved = await store.configureProvider({ label: provider.label, baseUrl: provider.baseUrl, model: provider.model, clearKey: true });
      setBaseUrl(saved.baseUrl); setModel(saved.model);
    } catch (failure) { setError(friendlyError(failure)); }
    finally { markBusy(false); }
  };
  return (
    <details className="advanced-settings">
      <summary><GearSix size={18} /><span>Model gateway setup</span><span className="disclosure-hint">Custom connection</span></summary>
      <form className="provider-form" onSubmit={event => { event.preventDefault(); void submit(); }}>
        <p className="field-note">Use an OpenAI-compatible connection you trust. Sending shares this chat’s messages with that connection.</p>
        {state.anyReplyPending && <p className="field-note">Finish or stop active replies before changing the connection.</p>}
        <fieldset disabled={!store.bridge.native || locked}>
          <label>Connection address<input type="url" value={baseUrl} maxLength={2048} placeholder="https://your-connection.example/v1" autoCapitalize="off" autoCorrect="off" spellCheck={false}
            onChange={event => { setBaseUrl(event.target.value); setChecked(false); }} required /></label>
          <label>Model <span className="optional">optional</span><input value={model} maxLength={200} list="forma-models" placeholder="Choose automatically after checking" autoCapitalize="off" spellCheck={false}
            onChange={event => { setModel(event.target.value); setChecked(false); }} /></label>
          <datalist id="forma-models">{baseUrl === provider.baseUrl && !apiKey && models.map(item => <option key={item} value={item} />)}</datalist>
          <button type="button" className="text-button muted" disabled={!provider.baseUrl || baseUrl !== provider.baseUrl || !!apiKey || state.modelsStatus === 'loading'}
            onClick={() => { void store.refreshModels().catch(() => {}); }}>{state.modelsStatus === 'loading' ? 'Refreshing models…' : 'Refresh available models'}</button>
          <p className="field-note">Leave Model blank to prefer the newest available Opus. Existing choices stay yours.</p>
          <label>API key <span className="optional">{provider.hasKey ? 'stored securely' : 'if required'}</span><input type="password" value={apiKey} maxLength={4096} placeholder={provider.hasKey ? 'Leave blank to keep your stored key' : 'Optional for keyless connections'}
            autoComplete="off" autoCapitalize="off" spellCheck={false} data-1p-ignore="true" data-lpignore="true" onChange={event => { setApiKey(event.target.value); setChecked(false); }} /></label>
          <div className="provider-actions"><button className="button secondary" type="submit" disabled={!baseUrl.trim()}>{busy ? 'Checking…' : changed ? 'Save & check connection' : 'Check connection'}</button>
            {provider.hasKey && <button type="button" className="text-button muted" onClick={() => void removeKey()}>Remove stored key</button>}</div>
        </fieldset>
        {state.modelsError && <InlineError>{state.modelsError}</InlineError>}
        {checked && provider.verified && <p className="connection-success" role="status"><CheckCircle size={17} />Connection checked · {provider.model}</p>}
        {error && <InlineError>{error}</InlineError>}
      </form>
    </details>
  );
}

export function Settings({ store, settings, browser, onClose }: { store: AppStore; settings: AppSettings; browser: OwnedBrowser; onClose: () => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState(settings.displayName);
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot);
  const save = async (patch: Parameters<AppStore['saveSettings']>[0]) => {
    setBusy(true); setError(null);
    try { await store.saveSettings(patch); } catch (failure) { setError(friendlyError(failure)); }
    finally { setBusy(false); }
  };
  return <Modal title="Settings" onClose={onClose} className="settings-modal" dismissible={!busy}>
    <section className="settings-section">
      <h3>Your space</h3>
      <form className="name-form" onSubmit={event => { event.preventDefault(); void save({ displayName: name.trim() }); }}>
        <label htmlFor="display-name">What should we call you? <span className="optional">optional</span></label>
        <div><input id="display-name" value={name} maxLength={80} autoComplete="given-name" placeholder="Your first name" onChange={event => setName(event.target.value)} disabled={busy} />
          <button className="button secondary" type="submit" disabled={busy || name.trim() === settings.displayName}>Update</button></div>
      </form>
      <label className="toggle-row"><span>Ambient motion<span className="row-detail">A little movement in your empty space</span></span>
        <input type="checkbox" role="switch" checked={settings.ambientMotion} disabled={busy} onChange={event => void save({ ambientMotion: event.target.checked })} /></label>
    </section>
    <section className="settings-section"><h3>Accounts</h3><Accounts browser={browser} /></section>
    <section className="settings-section">
      <div className="section-heading"><h3>Model & provider</h3><span className="availability">{settings.provider.verified ? 'Checked' : settings.provider.baseUrl ? 'Needs a check' : 'Not set up'}</span></div>
      {!store.bridge.native && <p className="browser-note"><LockSimple size={18} /><span>This browser preview saves chats and drafts locally. AI connections and secure keys are available in the desktop app.</span></p>}
      <ProviderSetup store={store} settings={settings} onBusyChange={setBusy} />
    </section>
    <RuntimeSettings store={store} />
    <AccountGrantStatus native={store.bridge.native} workspaceId={state.activeId} runtimeOrigin={store.bridge.native ? 'Forma-managed Hermes on this desktop' : undefined} modelOrigin={state.runtime?.capabilities?.modelOrigin} />
    <p className="settings-footnote">Chats are remembered automatically on this device. Deleting a workspace removes its chat and draft.</p>
    {error && <InlineError>{error}</InlineError>}
    <div className="modal-actions"><button type="button" className="button primary" disabled={busy} onClick={onClose}>Done</button></div>
  </Modal>;
}

export function Onboarding({ store, settings, browser, onSettings }: { store: AppStore; settings: AppSettings; browser: OwnedBrowser; onSettings: () => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const step = Math.min(2, Math.max(0, settings.onboardingStep));
  const advance = async (next: number) => {
    if (busy) return;
    setBusy(true); setError(null);
    try { await store.saveSettings(next > 2 ? { onboardingComplete: true, onboardingStep: 2 } : { onboardingStep: next }); }
    catch (failure) { setError(friendlyError(failure)); }
    finally { setBusy(false); }
  };
  return <main className="onboarding">
    <div className="onboarding-top"><Logo /></div>
    <section className="onboarding-card" aria-labelledby="onboarding-title">
      {step === 0 && <>
        <AmbientMedia enabled={settings.ambientMotion} className="welcome-media" />
        <div className="onboarding-copy"><span className="eyebrow">WELCOME TO FORMA</span><h1 id="onboarding-title">Set up your space.</h1><p>Keep your chats, browse your services, and choose the AI you use.</p></div>
      </>}
      {step === 1 && <div className="onboarding-copy accounts-step">
        <span className="step-symbol"><EnvelopeSimple size={27} /></span><h1 id="onboarding-title">Sign in to your services.</h1><p>Use your Forma browser. Its sessions stay separate from your normal browsing.</p>
        <Accounts browser={browser} /><ProviderSetup store={store} settings={settings} onBusyChange={setBusy} />
      </div>}
      {step === 2 && <div className="onboarding-copy start-step">
        <span className="step-symbol"><Sparkle size={29} /></span><h1 id="onboarding-title">Anything else to add?</h1><p>You can add more services now or come back from Settings.</p><button type="button" className="button secondary" disabled={busy || browser.busy} onClick={() => { void advance(1); }}>Add another service <ArrowRight size={16} /></button>
        <div className="readiness-card"><Check size={20} /><span>Local chats & drafts<span className="row-detail">Ready whenever you are</span></span></div>
        <div className="readiness-card"><GearSix size={20} /><span>Model provider<span className="row-detail">{settings.provider.verified ? 'Provider checked · Hermes status is in Settings' : 'Set up when you’re ready to send'}</span></span>
          {!settings.provider.verified && <button className="text-button" type="button" onClick={onSettings}>Settings <ArrowRight size={14} /></button>}</div>
      </div>}
      {error && <InlineError>{error}</InlineError>}
      <footer className="onboarding-footer">
        <div className="step-dots" aria-label={`Step ${step + 1} of 3`}>{[0, 1, 2].map(index => <span key={index} className={index === step ? 'current' : ''} />)}</div>
        <div>{step > 0 && <button className="text-button muted" type="button" disabled={busy} onClick={() => void advance(step - 1)}>Back</button>}
          <button className="button primary" type="button" disabled={busy} onClick={() => void advance(step + 1)}>{busy ? 'One moment…' : step === 2 ? 'Open chat' : step === 1 ? 'Continue' : 'Get started'}<ArrowRight size={17} /></button></div>
      </footer>
    </section>
  </main>;
}
