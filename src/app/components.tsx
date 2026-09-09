import { useEffect, useId, useRef, useState, type ReactNode } from 'react';
import { ArrowRight, Pause, Play, Sparkle, X } from '@phosphor-icons/react';
import { media } from './media';

export function IconButton({ label, children, onClick, disabled = false, className = '' }: {
  label: string; children: ReactNode; onClick: () => void; disabled?: boolean; className?: string;
}) {
  return <button type="button" className={`icon-button ${className}`} aria-label={label} title={label} onClick={onClick} disabled={disabled}>{children}</button>;
}

export function Modal({ title, children, onClose, className = '', dismissible = true }: {
  title: string; children: ReactNode; onClose: () => void; className?: string; dismissible?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const dialog = ref.current;
    dialog?.showModal();
    return () => { dialog?.close(); if (previous?.isConnected) previous.focus(); };
  }, []);
  return (
    <dialog ref={ref} className={`modal ${className}`} aria-labelledby={titleId}
      onCancel={event => { event.preventDefault(); if (dismissible) onCloseRef.current(); }}
      onClick={event => { if (dismissible && event.target === event.currentTarget) {
        const box = event.currentTarget.getBoundingClientRect();
        if (event.clientX < box.left || event.clientX > box.right || event.clientY < box.top || event.clientY > box.bottom) onCloseRef.current();
      } }}>
      <div className="modal-heading">
        <h2 id={titleId}>{title}</h2>
        {dismissible && <IconButton label="Close (Esc)" onClick={onClose}><X size={19} /></IconButton>}
      </div>
      {children}
    </dialog>
  );
}

export function SafeImage({ src, alt = '', className = '' }: { src: string | null; alt?: string; className?: string }) {
  const [failed, setFailed] = useState(false);
  return (
    <div className={`safe-image ${!src || failed ? 'image-fallback' : ''} ${className}`}>
      {src && !failed ? <img src={src} alt={alt} loading="lazy" onError={() => setFailed(true)} /> : <div className="quiet-art" aria-hidden="true"><span /><span /><span /><Sparkle size={25} weight="light" /></div>}
    </div>
  );
}

interface DataConnection extends EventTarget { saveData?: boolean }
export function AmbientMedia({ enabled, className = '' }: { enabled: boolean; className?: string }) {
  const video = useRef<HTMLVideoElement>(null);
  const [playing, setPlaying] = useState(false);
  const [failed, setFailed] = useState(false);
  const [allowed, setAllowed] = useState(false);
  const [pausedByUser, setPausedByUser] = useState(false);
  useEffect(() => {
    const reduced = matchMedia('(prefers-reduced-motion: reduce)');
    const connection = (navigator as Navigator & { connection?: DataConnection }).connection;
    const update = () => setAllowed(!reduced.matches && !connection?.saveData && document.visibilityState === 'visible');
    update();
    reduced.addEventListener('change', update);
    connection?.addEventListener('change', update);
    document.addEventListener('visibilitychange', update);
    return () => { reduced.removeEventListener('change', update); connection?.removeEventListener('change', update); document.removeEventListener('visibilitychange', update); };
  }, []);
  const shouldPlay = enabled && allowed && !pausedByUser && !failed;
  useEffect(() => {
    const element = video.current;
    if (!element) return;
    if (shouldPlay) void element.play().catch(() => { setPlaying(false); setPausedByUser(true); });
    else { element.pause(); setPlaying(false); }
  }, [shouldPlay]);
  return (
    <div className={`ambient-media ${className}`}>
      <SafeImage src={media.cozy} alt="Warm light falling across a quiet, comfortable room" />
      {media.ambient && enabled && allowed && !failed && <video ref={video} src={media.ambient} poster={media.poster ?? undefined}
        className={playing ? 'is-playing' : ''} muted loop playsInline preload="none" aria-hidden="true"
        onCanPlay={() => { if (shouldPlay) void video.current?.play().catch(() => setPausedByUser(true)); }}
        onPlay={() => setPlaying(true)} onPause={() => setPlaying(false)} onError={() => { setFailed(true); setPlaying(false); }} />}
      {media.ambient && enabled && allowed && !failed && <IconButton className="media-control" label={playing ? 'Pause ambient video' : 'Play ambient video'}
        onClick={() => setPausedByUser(!pausedByUser)}>{playing ? <Pause size={15} weight="fill" /> : <Play size={15} weight="fill" />}</IconButton>}
    </div>
  );
}

export function InlineError({ children, onRetry }: { children: ReactNode; onRetry?: () => void }) {
  return <div className="inline-error" role="alert"><span>{children}</span>{onRetry && <button type="button" className="text-button" onClick={onRetry}>Try again <ArrowRight size={14} /></button>}</div>;
}

export function Logo({ compact = false }: { compact?: boolean }) {
  return <span className={`forma-logo ${compact ? 'compact' : ''}`}><span className="logo-mark" aria-hidden="true"><Sparkle size={24} weight="fill" /></span>{!compact && <span>forma</span>}</span>;
}
