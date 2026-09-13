// Pure Composio service-surface contracts and helpers. No native-bridge
// imports: these feed both the rendered components and the regression checks.
export interface ComposioService {
  slug: string;
  name: string;
  description: string | null;
  logo: string | null;
  categories: string[];
  toolsCount: number | null;
  noAuth: boolean;
}
export interface ServiceCategory { id: string; name: string }
export interface CatalogResult { items: ComposioService[]; totalItems: number }
export type ServiceConnectionState = 'connected' | 'connecting' | 'needs_attention';
export interface ConnectedService {
  id: string;
  service: string;
  status: ServiceConnectionState;
  statusDetail: string | null;
  alias: string | null;
  wordId: string | null;
  connectedAt: string | null;
}
export interface ComposioAttempt {
  id: string;
  service: string;
  connectedAccountId: string | null;
  phase: 'pending' | 'confirming' | 'connected' | 'cancelled' | 'failed';
  expiresAt: string;
  error: { code: string; message: string } | null;
  detail: string | null;
}
export interface ServicesSnapshot {
  revision: number;
  keyConfigured: boolean;
  relay: boolean;
  attempt: ComposioAttempt | null;
  items: ConnectedService[];
}
export interface ServicePrompt { workspaceId: string; runId: string; service: string }
export interface LiveServicePrompt { prompt: ServicePrompt; dismissed: boolean }
export type ConnectCardPhase = 'idle' | 'connecting' | 'success' | 'failed';

export const CATALOG_INITIAL_ROWS = 10;
/** Upper bound for both the native fetch limit and rendered rows: "Show 10
 * more" pages through every fetched row (C1); the count line keeps the real
 * catalog total visible so truncation is never hidden. */
export const CATALOG_RENDER_MAX = 100;
export const CATALOG_STEP = 10;
export const MAX_PROMPT_CARDS = 3;

export interface CatalogSurface {
  loading: boolean;
  error: string | null;
  items: ComposioService[];
  totalItems: number;
  query: string;
  category: string;
  categories: ServiceCategory[];
  connectingService: string | null;
  connectedSlugs: readonly string[];
  onQuery(query: string): void;
  onCategory(id: string): void;
  onRetry(): void;
  onConnect(service: ComposioService): void;
  onCancelConnect(): void;
}
export function catalogCountLine(shown: number, total: number): string {
  return `Showing ${shown.toLocaleString()} of ${total.toLocaleString()} services`;
}
export function chipsFromCategories(categories: ServiceCategory[], max = 12): ServiceCategory[] {
  return [{ id: '', name: 'All' }, ...categories.slice(0, max - 1)];
}
export function serviceInitial(name: string): string {
  return Array.from(name.trim())[0]?.toLocaleUpperCase() ?? '?';
}
export function serviceDisplayName(slug: string): string {
  return slug.replace(/[_-]+/g, ' ').replace(/\b\w/g, character => character.toLocaleUpperCase());
}
export function serviceDetail(service: ComposioService): string {
  const category = service.categories[0];
  return category ? `${category} · Connects in your browser` : 'Connects in your browser';
}
export function connectionBadgeClass(status: ServiceConnectionState): string {
  if (status === 'connected') return 'connector-badge connector-badge--connected';
  if (status === 'connecting') return 'connector-badge connector-badge--connecting';
  return 'connector-badge connector-badge--error';
}
export function connectedIdentity(item: ConnectedService): string {
  return item.alias || item.wordId || item.service;
}
export function isAttemptLive(attempt: ComposioAttempt | null): boolean {
  return attempt?.phase === 'pending' || attempt?.phase === 'confirming';
}
export function connectCardPhase(
  service: string,
  attempt: ComposioAttempt | null,
  connected: readonly ConnectedService[],
): ConnectCardPhase {
  if (connected.some(item => item.service === service && item.status === 'connected')) return 'success';
  if (attempt && attempt.service === service) {
    if (isAttemptLive(attempt)) return 'connecting';
    if (attempt.phase === 'connected') return 'success';
    if (attempt.phase === 'failed') return 'failed';
  }
  return 'idle';
}
export function mergeServicePrompt(existing: ServicePrompt[], prompt: ServicePrompt): ServicePrompt[] {
  const kept = existing.filter(item =>
    !(item.service === prompt.service && item.workspaceId === prompt.workspaceId));
  const next = [...kept, prompt];
  return next.length > MAX_PROMPT_CARDS ? next.slice(next.length - MAX_PROMPT_CARDS) : next;
}
export function promptsForWorkspace(prompts: readonly ServicePrompt[], dismissed: readonly string[], workspaceId: string): LiveServicePrompt[] {
  return prompts
    .filter(prompt => prompt.workspaceId === workspaceId)
    .map(prompt => ({ prompt, dismissed: dismissed.includes(`${prompt.workspaceId}:${prompt.service}`) }));
}
export function validComposioKey(value: string): boolean {
  // Mirrors the native host rule exactly: ak_ prefix, 16..=4096 characters
  // total, printable ASCII (33-126) only — dots and other printable key
  // characters are accepted, whitespace and controls are not.
  const trimmed = value.trim();
  if (!trimmed.startsWith('ak_') || trimmed.length < 16 || trimmed.length > 4096) return false;
  for (let index = 0; index < trimmed.length; index++) {
    const code = trimmed.charCodeAt(index);
    if (code < 33 || code > 126) return false;
  }
  return true;
}
