import { useState, useSyncExternalStore } from 'react';
import { ArrowRight, Browser, CalendarBlank, Check, CheckCircle, EnvelopeSimple, GearSix, LockSimple, Sparkle } from '@phosphor-icons/react';
import type { AppSettings } from './contracts';
import type { AppStore } from './store';
import { friendlyError } from './bridge';
import { AmbientMedia, InlineError, Logo, Modal } from './components';

export function Accounts() {
  return <div className="accounts-list">
    <div className="account-row"><span className="account-icon"><EnvelopeSimple size={20} /></span><span>Google<span className="row-detail">Mail & calendar</span></span><span className="availability">Not available yet</span></div>
    <div className="account-row"><span className="account-icon"><CalendarBlank size={20} /></span><span>Apple<span className="row-detail">Calendar</span></span><span className="availability">Not available yet</span></div>
    <div className="account-row"><span className="account-icon"><Browser size={20} /></span><span>Your browser<span className="row-detail">A connected browsing space</span></span><span className="availability">Not available yet</span></div>
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
      <summary><GearSix size={18} /><span>Advanced AI setup</span><span className="disclosure-hint">Custom connection</span></summary>
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

export function Settings({ store, settings, onClose }: { store: AppStore; settings: AppSettings; onClose: () => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [name, setName] = useState(settings.displayName);
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
    <section className="settings-section"><h3>Accounts</h3><Accounts /></section>
    <section className="settings-section">
      <div className="section-heading"><h3>AI connection</h3><span className="availability">{settings.provider.verified ? 'Checked' : settings.provider.baseUrl ? 'Needs a check' : 'Not set up'}</span></div>
      {!store.bridge.native && <p className="browser-note"><LockSimple size={18} /><span>This browser preview saves chats and drafts locally. AI connections and secure keys are available in the desktop app.</span></p>}
      <ProviderSetup store={store} settings={settings} onBusyChange={setBusy} />
    </section>
    <p className="settings-footnote">Chats are remembered automatically on this device. Deleting a workspace removes its chat and draft.</p>
    {error && <InlineError>{error}</InlineError>}
    <div className="modal-actions"><button type="button" className="button primary" disabled={busy} onClick={onClose}>Done</button></div>
  </Modal>;
}

export function Onboarding({ store, settings, onSettings }: { store: AppStore; settings: AppSettings; onSettings: () => void }) {
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
    <div className="onboarding-top"><Logo /><span>YOUR OWN LITTLE CORNER</span></div>
    <section className="onboarding-card" aria-labelledby="onboarding-title">
      {step === 0 && <>
        <AmbientMedia enabled={settings.ambientMotion} className="welcome-media" />
        <div className="onboarding-copy"><span className="eyebrow">WELCOME TO FORMA</span><h1 id="onboarding-title">A little more room<br />to think.</h1><p>Your chats stay on this device. Bring a question, an idea, or a little everyday chaos.</p></div>
      </>}
      {step === 1 && <div className="onboarding-copy accounts-step">
        <span className="step-symbol"><EnvelopeSimple size={27} /></span><span className="eyebrow">MAKE YOURSELF AT HOME</span><h1 id="onboarding-title">Your world, at your pace.</h1><p>Start with what you share in chat. Account connections are on the way.</p>
        <Accounts /><ProviderSetup store={store} settings={settings} onBusyChange={setBusy} />
      </div>}
      {step === 2 && <div className="onboarding-copy start-step">
        <span className="step-symbol"><Sparkle size={29} /></span><span className="eyebrow">ALL SET TO EXPLORE</span><h1 id="onboarding-title">What’s on your mind?</h1><p>A fresh chat is waiting. Your work will be remembered as you go.</p>
        <div className="readiness-card"><Check size={20} /><span>Local chats & drafts<span className="row-detail">Ready whenever you are</span></span></div>
        <div className="readiness-card"><GearSix size={20} /><span>AI connection<span className="row-detail">{settings.provider.verified ? 'Checked and ready' : 'Set up when you’re ready to send'}</span></span>
          {!settings.provider.verified && <button className="text-button" type="button" onClick={onSettings}>Settings <ArrowRight size={14} /></button>}</div>
      </div>}
      {error && <InlineError>{error}</InlineError>}
      <footer className="onboarding-footer">
        <div className="step-dots" aria-label={`Step ${step + 1} of 3`}>{[0, 1, 2].map(index => <span key={index} className={index === step ? 'current' : ''} />)}</div>
        <div>{step > 0 && <button className="text-button muted" type="button" disabled={busy} onClick={() => void advance(step - 1)}>Back</button>}
          <button className="button primary" type="button" disabled={busy} onClick={() => void advance(step + 1)}>{busy ? 'One moment…' : step === 2 ? 'Open chat' : step === 1 ? 'Continue without accounts' : 'Make yourself at home'}<ArrowRight size={17} /></button></div>
      </footer>
    </section>
    <p className="onboarding-bottom">No signup. No rush.</p>
  </main>;
}
