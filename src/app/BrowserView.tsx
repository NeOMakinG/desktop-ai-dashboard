import { useEffect, useMemo, useRef, useState } from 'react';
import { ArrowLeft, ArrowRight, ArrowSquareOut, Browser, CalendarBlank, EnvelopeSimple, Globe, LockSimple, X } from '@phosphor-icons/react';
import { IconButton, InlineError } from './components';
import { defaultBrowserEngine, type BrowserEngine, type BrowserService } from './browser-contracts';
import type { OwnedBrowser } from './owned-browser';
import './browser.css';

const services = [
  { id: 'gmail' as const, name: 'Gmail', detail: 'Mail', icon: EnvelopeSimple },
  { id: 'google_calendar' as const, name: 'Google Calendar', detail: 'Your schedule', icon: CalendarBlank },
];

export function BrowserConnections({ browser, openWithEngine }: {
  browser: OwnedBrowser;
  // Optional so callers outside BrowserView (onboarding, etc.) keep working
  // without threading engine state through — they get the WebKit default.
  openWithEngine?: (service: BrowserService, url?: string) => Promise<void>;
}) {
  const [showCustom, setShowCustom] = useState(false);
  const [customUrl, setCustomUrl] = useState('');
  const enabled = browser.native && browser.status.available && !browser.busy;
  const open = (service: BrowserService, url?: string) => openWithEngine
    ? void openWithEngine(service, url)
    : void browser.open(service, url);
  return <div className="browser-connections" role="group" aria-label="Website browsing">
    <p className="field-note">Website browsing in Forma is separate from connecting Google. These websites do not give the agent Gmail or Calendar access.</p>
    {services.map(service => <div className="account-row" key={service.id}>
      <span className="account-icon"><service.icon size={20} /></span>
      <span>{service.name}<span className="row-detail">{service.detail}</span></span>
      <button type="button" className="button secondary" disabled={!enabled} onClick={() => open(service.id)}>Open website <ArrowSquareOut size={14} /></button>
    </div>)}
    <div className="account-row">
      <span className="account-icon"><Globe size={20} /></span>
      <span>Another service<span className="row-detail">Open it in your Forma browser</span></span>
      <button type="button" className="button secondary" disabled={!enabled} aria-expanded={showCustom} onClick={() => setShowCustom(value => !value)}>Add <ArrowSquareOut size={14} /></button>
    </div>
    {showCustom && <form className="browser-add-service" onSubmit={event => {
      event.preventDefault();
      if (enabled && customUrl.trim()) open('custom', customUrl.trim());
    }}><label>Website address<input autoFocus type="url" required value={customUrl} maxLength={2048} placeholder="https://…" disabled={!enabled} onChange={event => setCustomUrl(event.target.value)} /></label><button type="submit" className="button secondary" disabled={!enabled || !customUrl.trim()}>Open website</button></form>}
    {!browser.native && <p className="field-note">Open websites from the desktop app.</p>}
    {browser.native && !browser.status.available && <p className="field-note">{browser.status.error || 'The owned browser is not available on this build.'}</p>}
    {browser.busy && <p className="field-note" role="status">Opening your browser…</p>}
    {browser.error && <InlineError>{browser.error}</InlineError>}
  </div>;
}

export function BrowserView({ browser }: { browser: OwnedBrowser }) {
  const [address, setAddress] = useState('');
  const editing = useRef(false);
  const open = browser.status.phase === 'open';
  const enabled = browser.native && browser.status.available && !browser.busy;
  useEffect(() => { if (!editing.current) setAddress(browser.status.url || ''); }, [browser.status.url]);

  // Engine toggle. Only rendered when the host actually advertises more than
  // one engine — a single engine has nothing to toggle to. The default choice
  // is the host's advertised default (Scrapling first when its sealed
  // resources verify; WebKit otherwise) — never a silent swap.
  const engines = useMemo(() => browser.status.availableEngines ?? [], [browser.status.availableEngines]);
  const hostDefault = useMemo(() => defaultBrowserEngine(engines), [engines]);
  const scrapling = engines.find(engine => engine.id === 'scrapling');
  const webkit = engines.find(engine => engine.id === 'webkit');
  const showToggle = !!webkit && !!scrapling;
  const [engineChoice, setEngineChoice] = useState<BrowserEngine>(hostDefault === 'scrapling' ? 'scrapling' : 'webkit');
  useEffect(() => {
    // If the host stops advertising the chosen engine, fall back to the
    // advertised default rather than leaving the toggle on something gone.
    if (!engines.some(engine => engine.id === engineChoice)) {
      setEngineChoice(hostDefault === 'scrapling' ? 'scrapling' : 'webkit');
    }
  }, [engines, hostDefault, engineChoice]);

  const openWithEngine = async (service: BrowserService, url?: string) => {
    if (!enabled) return;
    await browser.open(service, url, engines.some(engine => engine.id === engineChoice) ? engineChoice : undefined);
  };
  // Scrapling sessions render fetched snapshots in the browser window: honest
  // read-only and fetching states, never a pretend-live page.
  const scraplingOpen = browser.status.engine === 'scrapling' && open;

  return <section className="browser-page" aria-labelledby="browser-title">
    <header className="browser-heading">
      <span className="browser-symbol"><Browser size={30} /></span>
      <div><h1 id="browser-title">Your browser</h1><p>A separate place for the services you use with Forma.</p></div>
    </header>
    <form className="browser-toolbar" onSubmit={event => {
      event.preventDefault();
      const value = address.trim();
      if (value && enabled) {
        editing.current = false;
        if (open) void browser.navigate(value);
        else void openWithEngine('custom', value);
      }
    }}>
      <div className="browser-history-controls">
        <IconButton label="Back in browser" disabled={!open || !enabled} onClick={() => { void browser.back(); }}><ArrowLeft size={18} /></IconButton>
        <IconButton label="Forward in browser" disabled={!open || !enabled} onClick={() => { void browser.forward(); }}><ArrowRight size={18} /></IconButton>
        <button type="button" className="text-button muted" disabled={!open || !enabled} onClick={() => { void browser.reload(); }}>Reload</button>
      </div>
      <label className="browser-address"><Globe size={17} /><input aria-label="Website address" type="url" value={address} maxLength={2048} placeholder="https://…" autoCapitalize="off" autoCorrect="off" spellCheck={false}
        onFocus={() => { editing.current = true; }} onBlur={() => { editing.current = false; }} onChange={event => setAddress(event.target.value)} disabled={!enabled} /></label>
      <button type="submit" className="button secondary" disabled={!enabled || !address.trim()}>Go</button>
    </form>
    {browser.error && <InlineError>{browser.error}</InlineError>}
    {browser.status.navigating && open && <p className="field-note" role="status">Fetching the page through the Scrapling engine…</p>}
    {scraplingOpen && <p className="field-note">Read-only snapshots: pages are fetched by Scrapling's engine and shown without live scripts. Typing and signing in inside this window are not supported yet.</p>}
    <div className="browser-home-card">
      <div className="browser-window-preview" aria-hidden="true"><div className="browser-preview-chrome"><i /><i /><i /><span /></div><div className="browser-preview-content"><Browser size={44} weight="light" /><span>Your own browsing space</span></div></div>
      <div className="browser-home-copy"><h2>{open ? 'Your browser is open' : 'Sign in once. Keep your sessions.'}</h2>
        <p>{open ? 'Continue in the Forma browser window, or open another service below.' : 'Forma uses its own browser profile. It does not copy the sessions from your normal browser.'}</p>
        {showToggle && scrapling && (
          <div className="browser-engine-toggle" role="radiogroup" aria-label="Browser engine">
            <button type="button" role="radio" aria-checked={engineChoice === 'scrapling'} className={`browser-engine-option${engineChoice === 'scrapling' ? ' selected' : ''}`} onClick={() => setEngineChoice('scrapling')} disabled={!enabled || open}>Scrapling</button>
            <button type="button" role="radio" aria-checked={engineChoice === 'webkit'} className={`browser-engine-option${engineChoice === 'webkit' ? ' selected' : ''}`} onClick={() => setEngineChoice('webkit')} disabled={!enabled || open}>WebKit</button>
          </div>
        )}
        <button type="button" className="button primary" disabled={!enabled} onClick={() => { void openWithEngine('home'); }}>{browser.busy ? 'Opening…' : open ? 'Show browser' : 'Open browser'} <ArrowSquareOut size={17} /></button>
        {(open || browser.status.phase === 'opening') && <button type="button" className="text-button muted" onClick={() => { void browser.close(); }} disabled={browser.actionPending}><X size={15} />{open ? 'Close window' : 'Cancel opening'}</button>}
      </div>
    </div>
    <section className="browser-service-section" aria-labelledby="browser-services-title"><h2 id="browser-services-title">Browse websites</h2><BrowserConnections browser={browser} openWithEngine={openWithEngine} /></section>
    {browser.status.chromiumUnavailableReason && <p className="field-note">{browser.status.chromiumUnavailableReason}</p>}
    <details className="browser-details"><summary><LockSimple size={16} />About this browser</summary><p>Closing the window keeps its site sessions on this device. API access requires each service’s supported authorization; signing in does not create API keys. Browser automation is not connected to the agent yet.</p><p>Some services require sign-in through a supported external browser. Forma will not bypass their login requirements.</p></details>
  </section>;
}
