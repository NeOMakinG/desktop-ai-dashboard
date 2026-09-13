import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ArrowLeft, ArrowRight, ArrowSquareOut, ArrowsInSimple, LockSimple, Plus, Warning, X } from '@phosphor-icons/react';
import { IconButton } from './components';
import {
  cdpModifiers, cdpMouseButton, embeddedContainRect, embeddedPointToPage, embeddedShortcut,
  embeddedTabLabel, emptyEmbeddedState, isSecureUrl, normalizeAddress,
  type EmbeddedFrameMeta, type EmbeddedState, type FitRect,
} from './browser-contracts';

interface FramePayload {
  tab: string;
  /** Base64 JPEG straight from Page.screencastFrame — decoded on a canvas. */
  data: string;
  metadata: EmbeddedFrameMeta & Record<string, unknown>;
}

/** The real Chrome browser, experienced entirely inside the Forma window:
 * Forma's own tab strip and address bar, a CDP screencast canvas, and full
 * input forwarding. The Chrome window itself sits offscreen until popped out.
 * Fills the whole right panel edge-to-edge (founder directive). */
export function EmbeddedChrome({ onError, assistantDrive, onAssistantDrive }: {
  onError: (message: string | null) => void;
  assistantDrive: boolean;
  onAssistantDrive: (enabled: boolean) => void;
}) {
  const [state, setState] = useState<EmbeddedState>({ ...emptyEmbeddedState, running: true });
  const [address, setAddress] = useState('');
  const [focused, setFocused] = useState(false);
  const editing = useRef(false);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const frameRef = useRef<HTMLDivElement | null>(null);
  const meta = useRef<EmbeddedFrameMeta | null>(null);
  const rect = useRef<FitRect>({ x: 0, y: 0, width: 0, height: 0 });
  const activeId = useRef<string | null>(null);
  const hasFrame = useRef(false);
  activeId.current = state.activeId;

  const activeTab = state.tabs.find(tab => tab.active) ?? null;
  useEffect(() => {
    if (!editing.current) setAddress(activeTab?.url && activeTab.url !== 'about:blank' ? activeTab.url : '');
  }, [activeTab?.url]);

  const run = useCallback(async (command: string, args?: Record<string, unknown>) => {
    try {
      const next = await invoke<EmbeddedState | null>(command, args);
      if (next && typeof next === 'object' && 'tabs' in next) setState(next);
      onError(null);
    } catch {
      // Transient failures (a command racing a CDP reconnect) are not shown as
      // errors — the state stream drives the UI, and reconnect is automatic.
      // A genuine browser exit flips running=false and unmounts this view.
    }
  }, [onError]);

  // State + frame subscriptions. Frames draw straight onto the canvas —
  // contain-fit, devicePixelRatio aware — and remember the page geometry so
  // pointer events map back into page CSS coordinates.
  useEffect(() => {
    let disposed = false;
    const releases: Array<() => void> = [];
    void invoke<EmbeddedState>('browser_embedded_state').then(next => { if (!disposed) setState(next); }).catch(() => {});
    void listen<EmbeddedState>('embedded-browser:state', event => { if (!disposed) setState(event.payload); })
      .then(release => { if (disposed) release(); else releases.push(release); }).catch(() => {});
    void listen<FramePayload>('embedded-browser:frame', event => {
      if (disposed || event.payload.tab !== activeId.current) return;
      const canvas = canvasRef.current;
      if (!canvas) return;
      const image = new Image();
      image.onload = () => {
        if (disposed) return;
        const dpr = window.devicePixelRatio || 1;
        const cssW = canvas.clientWidth;
        const cssH = canvas.clientHeight;
        if (canvas.width !== Math.round(cssW * dpr) || canvas.height !== Math.round(cssH * dpr)) {
          canvas.width = Math.round(cssW * dpr);
          canvas.height = Math.round(cssH * dpr);
        }
        const context = canvas.getContext('2d');
        if (!context) return;
        const fit = embeddedContainRect(cssW, cssH, image.naturalWidth, image.naturalHeight);
        rect.current = fit;
        meta.current = {
          deviceWidth: Number(event.payload.metadata?.deviceWidth) || image.naturalWidth,
          deviceHeight: Number(event.payload.metadata?.deviceHeight) || image.naturalHeight,
        };
        context.setTransform(dpr, 0, 0, dpr, 0, 0);
        context.clearRect(0, 0, cssW, cssH);
        context.drawImage(image, fit.x, fit.y, fit.width, fit.height);
        hasFrame.current = true;
      };
      image.src = `data:image/jpeg;base64,${event.payload.data}`;
    }).then(release => { if (disposed) release(); else releases.push(release); }).catch(() => {});
    return () => { disposed = true; releases.forEach(release => release()); };
  }, []);

  // Report the surface size so the host scales the screencast to it.
  useEffect(() => {
    const element = frameRef.current;
    if (!element) return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const report = () => {
      const width = Math.round(element.clientWidth);
      const height = Math.round(element.clientHeight);
      if (width && height) void invoke('browser_embedded_viewport', { width, height, scale: window.devicePixelRatio || 1 }).catch(() => {});
    };
    const observer = new ResizeObserver(() => { clearTimeout(timer); timer = setTimeout(report, 250); });
    observer.observe(element);
    report();
    return () => { clearTimeout(timer); observer.disconnect(); };
  }, []);

  const forward = useCallback((event: Record<string, unknown>) => {
    void invoke('browser_embedded_input', { event }).catch(() => {});
  }, []);

  const pointer = useCallback((raw: React.MouseEvent, type: 'mousePressed' | 'mouseReleased' | 'mouseMoved') => {
    const canvas = canvasRef.current;
    const geometry = meta.current;
    if (!canvas || !geometry) return null;
    const bounds = canvas.getBoundingClientRect();
    const point = embeddedPointToPage(raw.clientX - bounds.left, raw.clientY - bounds.top, rect.current, geometry);
    if (!point) return null;
    return {
      kind: 'mouse', type, x: point.x, y: point.y,
      button: type === 'mouseMoved' ? undefined : cdpMouseButton(raw.button) ?? undefined,
      clickCount: type === 'mouseMoved' ? undefined : (raw.detail || 1),
      modifiers: cdpModifiers(raw),
    };
  }, []);

  const paste = useCallback(() => {
    void navigator.clipboard?.readText?.().then(text => {
      if (text) forward({ kind: 'paste', text });
    }).catch(() => {});
  }, [forward]);

  const onKey = useCallback((raw: React.KeyboardEvent, type: 'keyDown' | 'keyUp') => {
    const shortcut = embeddedShortcut(raw);
    if (shortcut) {
      raw.preventDefault();
      if (type !== 'keyDown') return;
      if (shortcut === 'new_tab') void run('browser_embedded_new_tab');
      else if (shortcut === 'close_tab' && activeId.current) void run('browser_embedded_close_tab', { target: activeId.current });
      else if (shortcut === 'paste') paste();
      return;
    }
    // Let app-level shortcuts (Cmd+Q etc.) through untouched; forward the rest.
    if (raw.metaKey && !['a', 'c', 'x', 'z', 'y'].includes(raw.key.toLowerCase())) return;
    raw.preventDefault();
    forward({
      kind: 'key', type, key: raw.key.length <= 32 ? raw.key : undefined, code: raw.code,
      text: type === 'keyDown' && raw.key.length === 1 && !raw.metaKey && !raw.ctrlKey ? raw.key : undefined,
      windowsVirtualKeyCode: raw.keyCode || undefined, modifiers: cdpModifiers(raw), autoRepeat: raw.repeat,
    });
  }, [forward, paste, run]);

  const popped = state.poppedOut;
  return <div className="embedded-chrome" data-popped={popped || undefined}>
    <div className="embedded-tabstrip" role="tablist" aria-label="Browser tabs">
      {state.tabs.map(tab => (
        <div key={tab.id} className={`embedded-tab${tab.active ? ' active' : ''}`} role="tab" aria-selected={tab.active}>
          <button type="button" className="embedded-tab-label" title={tab.url} onClick={() => { void run('browser_embedded_activate_tab', { target: tab.id }); }}>
            {tab.loading ? <span className="embedded-tab-spinner" aria-label="Loading" /> : null}
            <span>{embeddedTabLabel(tab)}</span>
          </button>
          <IconButton label={`Close tab ${embeddedTabLabel(tab)}`} className="embedded-tab-close" onClick={() => { void run('browser_embedded_close_tab', { target: tab.id }); }}><X size={12} /></IconButton>
        </div>
      ))}
      <IconButton label="New tab (Cmd+T)" className="embedded-newtab" onClick={() => { void run('browser_embedded_new_tab'); }}><Plus size={15} /></IconButton>
    </div>
    <form className="embedded-toolbar" onSubmit={event => {
      event.preventDefault();
      const target = normalizeAddress(address);
      if (target) { editing.current = false; void run('browser_embedded_navigate', { url: target }); }
    }} onKeyDown={event => {
      // Belt and braces: Enter submits even if implicit submission is skipped.
      if (event.key === 'Enter') {
        event.preventDefault();
        const target = normalizeAddress(address);
        if (target) { editing.current = false; void run('browser_embedded_navigate', { url: target }); }
      }
    }}>
      <IconButton label="Back" onClick={() => { void run('browser_embedded_history', { action: 'back' }); }}><ArrowLeft size={16} /></IconButton>
      <IconButton label="Forward" onClick={() => { void run('browser_embedded_history', { action: 'forward' }); }}><ArrowRight size={16} /></IconButton>
      <button type="button" className="text-button muted" onClick={() => { void run('browser_embedded_history', { action: 'reload' }); }}>Reload</button>
      <label className="browser-address embedded-address">
        {isSecureUrl(activeTab?.url) ? <LockSimple size={15} aria-label="Secure connection" /> : <Warning size={15} aria-label="Not a secure page" />}
        <input aria-label="Website address" type="text" value={address} maxLength={2048} placeholder="Search is not wired — enter a website address"
          autoCapitalize="off" autoCorrect="off" spellCheck={false}
          onFocus={event => { editing.current = true; event.currentTarget.select(); }} onBlur={() => { editing.current = false; }}
          onChange={event => setAddress(event.target.value)} />
      </label>
      <button type="submit" className="button secondary" disabled={!normalizeAddress(address)}>Go</button>
      <label className="embedded-drive" title="When on, the assistant can control this Chrome while it is open. Off by default.">
        <input type="checkbox" role="switch" checked={assistantDrive} onChange={event => onAssistantDrive(event.target.checked)} />
        <span>Assistant drive</span>
      </label>
      <button type="button" className="text-button muted embedded-pop" onClick={() => { void run('browser_embedded_pop', { out: !popped }); }}>
        {popped ? <><ArrowsInSimple size={15} /> Bring back in</> : <><ArrowSquareOut size={15} /> Pop out</>}
      </button>
    </form>
    {state.running && !state.connected && !popped && (
      <div className="embedded-reconnect" role="status">
        <span className="embedded-tab-spinner" aria-hidden="true" /> Reconnecting to Chrome…
        <button type="button" className="text-button muted" onClick={() => { void run('browser_embedded_state'); }}>Retry now</button>
      </div>
    )}
    <div ref={frameRef} className="embedded-frame">
      {popped ? (
        <div className="embedded-placeholder" role="status">
          <p>Chrome is popped out into its own window.</p>
          <button type="button" className="button secondary" onClick={() => { void run('browser_embedded_pop', { out: false }); }}>Bring it back into Forma</button>
        </div>
      ) : (
        <>
          <canvas
            ref={canvasRef}
            className="embedded-canvas"
            tabIndex={0}
            aria-label="Embedded Chrome page. Click to interact; keys are sent to the page while focused."
            onFocus={() => setFocused(true)}
            onBlur={() => { setFocused(false); }}
            onMouseDown={event => { canvasRef.current?.focus(); const payload = pointer(event, 'mousePressed'); if (payload) { event.preventDefault(); forward(payload); } }}
            onMouseUp={event => { const payload = pointer(event, 'mouseReleased'); if (payload) forward(payload); }}
            onMouseMove={event => { const payload = pointer(event, 'mouseMoved'); if (payload) forward(payload); }}
            onWheel={event => {
              const payload = pointer(event, 'mouseMoved');
              if (payload) forward({ ...payload, type: 'mouseWheel', deltaX: event.deltaX, deltaY: event.deltaY });
            }}
            onKeyDown={event => onKey(event, 'keyDown')}
            onKeyUp={event => onKey(event, 'keyUp')}
            onPaste={event => { const text = event.clipboardData.getData('text'); if (text) { event.preventDefault(); forward({ kind: 'paste', text }); } }}
            onContextMenu={event => event.preventDefault()}
          />
          {!focused && <button type="button" className="embedded-focus-hint" onClick={() => canvasRef.current?.focus()}>Click to interact</button>}
        </>
      )}
    </div>
  </div>;
}
