import { useEffect, useRef, useState, type RefObject } from 'react';
import { ArrowRight, ArrowSquareOut, CheckCircle, MagnifyingGlass, Plugs, WarningCircle, X } from '@phosphor-icons/react';
import { InlineError, SafeImage } from './components';
import type { ServicesController } from './services-controller';
import {
  CATALOG_INITIAL_ROWS,
  CATALOG_RENDER_CAP,
  CATALOG_STEP,
  catalogCountLine,
  chipsFromCategories,
  connectionBadgeClass,
  connectedIdentity,
  isAttemptLive,
  serviceDetail,
  serviceDisplayName,
  serviceInitial,
  type CatalogSurface,
  type ComposioService,
  type ConnectCardPhase,
  type ConnectedService,
  type ServiceCategory,
} from './services-contracts';
import './services.css';

export function ServiceSearch({ query, onQuery, searchRef }: { query: string; onQuery: (query: string) => void; searchRef?: RefObject<HTMLInputElement | null> }) {
  return <div className="browser-address service-search">
    <MagnifyingGlass size={17} />
    <input ref={searchRef} value={query} aria-label="Search services" placeholder="Search services…"
      maxLength={120} autoCapitalize="off" autoCorrect="off" spellCheck={false} onChange={event => onQuery(event.target.value)} />
  </div>;
}

export function ServiceChips({ categories, selected, onSelect }: { categories: ServiceCategory[]; selected: string; onSelect: (id: string) => void }) {
  return <div className="service-chips" role="group" aria-label="Service categories">
    {chipsFromCategories(categories).map(chip => (
      <button key={chip.id || 'all'} type="button" className={`service-chip ${selected === chip.id ? 'selected' : ''}`}
        aria-pressed={selected === chip.id} onClick={() => onSelect(chip.id)}>{chip.name}</button>
    ))}
  </div>;
}

export function ServiceRow({ service, connecting, onCancelConnect, onConnect }: {
  service: ComposioService; connecting: boolean; onCancelConnect: () => void; onConnect: (service: ComposioService) => void;
}) {
  return <div className="account-row service-row">
    <span className="account-icon">
      {service.logo ? <SafeImage src={service.logo} alt="" /> : <span className="service-initial" aria-hidden="true">{serviceInitial(service.name)}</span>}
    </span>
    <span className="service-name">{service.name}<span className="row-detail">{serviceDetail(service)}</span></span>
    {connecting ? <span className="service-connect-area" role="status">
      <button type="button" className="button secondary service-connect-button is-connecting" disabled>Connecting…</button>
      <button type="button" className="text-button" onClick={onCancelConnect}>Cancel</button>
    </span> : <button type="button" className="button secondary service-connect-button" onClick={() => onConnect(service)}>Connect <ArrowSquareOut size={14} /></button>}
  </div>;
}

export function ServiceResults({ surface }: { surface: CatalogSurface }) {
  const [shown, setShown] = useState(CATALOG_INITIAL_ROWS);
  useEffect(() => { setShown(CATALOG_INITIAL_ROWS); }, [surface.query, surface.category]);
  const visible = surface.items.slice(0, Math.min(shown, CATALOG_RENDER_CAP));
  const { query, error, loading, items } = surface;
  return <div className="service-results">
    {!query && !error && !loading && items.length > 0 && <p className="service-list-heading">Popular services</p>}
    {error && <InlineError onRetry={surface.onRetry}>{error}</InlineError>}
    {loading && items.length === 0 && <><span className="sr-only" role="status">Loading services</span>
      {[0, 1, 2, 3, 4, 5].map(row => <div key={row} className="skeleton service-skeleton" aria-hidden="true" />)}</>}
    {!loading && !error && items.length === 0 && (query
      ? <p className="search-empty">No services match “{query}”. Try a shorter name, or clear your search.
        <button type="button" className="text-button" onClick={() => surface.onQuery('')}>Clear search</button></p>
      : <p className="search-empty">No services are available right now. Try again in a moment.</p>)}
    {items.length > 0 && <div className="service-catalog" role="region" aria-label="Service catalog">
      {visible.map(service => <ServiceRow key={service.slug} service={service}
        connecting={surface.connectingService === service.slug}
        onCancelConnect={surface.onCancelConnect} onConnect={surface.onConnect} />)}
      {shown < Math.min(items.length, CATALOG_RENDER_CAP) && <button type="button" className="text-button muted service-more" onClick={() => setShown(value => value + CATALOG_STEP)}>Show 10 more</button>}
      <p className="service-count">{catalogCountLine(visible.length, surface.totalItems)}</p>
    </div>}
  </div>;
}

export function ServiceCatalog({ surface, searchRef }: { surface: CatalogSurface; searchRef?: RefObject<HTMLInputElement | null> }) {
  return <div className="service-catalog-surface">
    <ServiceSearch query={surface.query} onQuery={surface.onQuery} searchRef={searchRef} />
    <ServiceChips categories={surface.categories} selected={surface.category} onSelect={surface.onCategory} />
    <ServiceResults surface={surface} />
  </div>;
}

export function ConnectedServiceRow({ item, busy, onDisconnect }: {
  item: ConnectedService; busy: boolean; onDisconnect: (id: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const anchor = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!open) return;
    const handler = (event: MouseEvent) => { if (anchor.current && !anchor.current.contains(event.target as Node)) setOpen(false); };
    const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') { setOpen(false); anchor.current?.querySelector('button')?.focus(); } };
    document.addEventListener('mousedown', handler);
    document.addEventListener('keydown', escape, true);
    return () => { document.removeEventListener('mousedown', handler); document.removeEventListener('keydown', escape, true); };
  }, [open]);
  return <div className="account-row connector-row service-row">
    <span className="account-icon">
      <span className="service-initial" aria-hidden="true">{serviceInitial(connectedIdentity(item))}</span>
    </span>
    <span>{serviceDisplayName(item.service)}
      <span className="row-detail" role="status" aria-live="polite">{connectedIdentity(item)}{item.statusDetail ? ` · ${item.statusDetail}` : ''}</span>
    </span>
    {item.status === 'connected' && <span className={connectionBadgeClass('connected')}><CheckCircle size={13} weight="fill" />Connected</span>}
    {item.status === 'connecting' && <span className={connectionBadgeClass('connecting')}><span className="thinking-dots" aria-hidden="true"><i /><i /><i /></span>Connecting…</span>}
    {item.status === 'needs_attention' && <span className={connectionBadgeClass('needs_attention')}><WarningCircle size={13} />Needs attention</span>}
    <div className="connector-menu-anchor" ref={anchor}>
      <button type="button" className="text-button muted" aria-haspopup="menu" aria-expanded={open} disabled={busy}
        onClick={() => setOpen(value => !value)}>Manage</button>
      {open && <div className="connector-popover" role="menu">
        <button type="button" className="text-button danger-text" role="menuitem" disabled={busy}
          onClick={() => { setOpen(false); onDisconnect(item.id); }}><X size={14} />Disconnect</button>
      </div>}
    </div>
  </div>;
}

export function ServiceConnectCard({ service, phase, reason, identity, error, onConnect, onCancelConnect, onDismiss, onRetry, onManage }: {
  service: string; phase: ConnectCardPhase | 'dismissed'; reason: string; identity: string | null; error: string | null;
  onConnect: () => void; onCancelConnect: () => void; onDismiss: () => void; onRetry: () => void; onManage: () => void;
}) {
  const name = serviceDisplayName(service);
  if (phase === 'success') {
    return <p className="connection-success service-card-success" role="status"><CheckCircle size={13} />Connected · {name}{identity ? ` · ${identity}` : ''}
      <button type="button" className="text-button" onClick={onManage}>Manage in Services</button></p>;
  }
  if (phase === 'dismissed') {
    return <p className="service-card-later"><span>You chose to connect {name} later.</span>
      <button type="button" className="text-button" onClick={onConnect}>Connect now</button></p>;
  }
  return <article className="service-card-message">
    <section className="connect-card" aria-label={`Connect ${name}`}>
      <span className="sr-only" role="status">{`Forma needs ${name}. A connect button is available.`}</span>
      <header className="connect-card-header">
        <span className="account-icon"><span className="service-initial" aria-hidden="true">{serviceInitial(name)}</span></span>
        <span className="connect-card-title">Connect {name}</span>
      </header>
      <p className="connect-card-body">{reason}</p>
      <p className="field-note">Opens your default browser. Forma sees only what you approve, and you can disconnect anytime.</p>
      {phase === 'failed' && <InlineError onRetry={onRetry}>{error || 'The connection could not start.'}</InlineError>}
      <div className="connect-actions">
        {phase === 'connecting'
          ? <><button type="button" className="button primary service-connect-button is-connecting" disabled>Connecting…</button>
            <button type="button" className="text-button" onClick={onCancelConnect}>Cancel</button></>
          : <><button type="button" className="button primary" onClick={onConnect}>Connect {name} <ArrowSquareOut size={14} /></button>
            <button type="button" className="text-button muted" onClick={onDismiss}>Not now</button></>}
      </div>
    </section>
  </article>;
}

export function ServicesPageSymbol() {
  return <span className="browser-symbol"><Plugs size={30} /></span>;
}

export function catalogSurface(controller: ServicesController): CatalogSurface {
  const attempt = controller.snapshot?.attempt ?? null;
  return {
    loading: controller.catalog.loading,
    error: controller.catalog.error,
    items: controller.catalog.items,
    totalItems: controller.catalog.totalItems,
    query: controller.query,
    category: controller.category,
    categories: controller.categories,
    connectingService: attempt && isAttemptLive(attempt) ? attempt.service : null,
    connectedSlugs: (controller.snapshot?.items ?? [])
      .filter(item => item.status === 'connected')
      .map(item => item.service),
    onQuery: controller.setQuery,
    onCategory: controller.setCategory,
    onRetry: controller.retryCatalog,
    onConnect: service => { void controller.connect(service.slug); },
    onCancelConnect: () => { if (attempt) void controller.cancelConnect(attempt.id); },
  };
}

export function ServicesView({ controller, onSettings }: { controller: ServicesController; onSettings: () => void }) {
  const search = useRef<HTMLInputElement | null>(null);
  const connected = controller.snapshot?.items ?? [];
  const attempt = controller.snapshot?.attempt ?? null;
  const keyConfigured = controller.snapshot?.keyConfigured ?? false;
  return <div className="browser-page services-page" aria-labelledby="services-title">
    <header className="browser-heading"><ServicesPageSymbol />
      <div><h1 id="services-title">Services</h1>
        <p>Accounts Forma can use. Each connection is your explicit choice; Forma asks again before anything is sent.</p></div>
    </header>
    {controller.statusError && <div className="services-section">
      <InlineError onRetry={() => void controller.refreshStatus()}>{controller.statusError}</InlineError>
    </div>}
    {!keyConfigured && <div className="services-section">
      <p className="field-note">Add your Composio API key in Settings to browse and connect services.</p>
      <button type="button" className="text-button" onClick={onSettings}>Open Settings <ArrowRight size={14} /></button>
    </div>}
    <div className="services-toolbar">
      <ServiceSearch query={controller.query} onQuery={controller.setQuery} searchRef={search} />
      <ServiceChips categories={controller.categories} selected={controller.category} onSelect={controller.setCategory} />
    </div>
    <section className="services-section" aria-labelledby="services-connected-heading">
      <h2 id="services-connected-heading">Connected <span className="services-count">{connected.length}</span></h2>
      {connected.length === 0 && !controller.statusError && <div className="readiness-card service-empty-card">
        <Plugs size={20} /><span>No services connected yet<span className="row-detail">Choose a service below when you’re ready</span></span>
        <button type="button" className="button secondary" onClick={() => search.current?.focus()}>Browse services</button>
      </div>}
      {connected.map(item => <ConnectedServiceRow key={item.id} item={item} busy={isAttemptLive(attempt)}
        onDisconnect={id => { void controller.disconnect(id); }} />)}
    </section>
    <section className="services-section" aria-label="Available services">
      <h2>Available</h2>
      <ServiceResults surface={catalogSurface(controller)} />
      {controller.connectError && <InlineError>{controller.connectError}</InlineError>}
    </section>
  </div>;
}
