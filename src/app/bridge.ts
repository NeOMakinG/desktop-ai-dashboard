import { invoke, isTauri } from '@tauri-apps/api/core';
import type {
  AppBridge, AppSettings, Bootstrap, ChatMessage, ChatWorkspace, ProviderConfig,
  ProviderInput, SettingsInput, WorkspaceSummary,
} from './contracts';

export const BROWSER_STORAGE_KEY = 'forma.chat.v1';
const emptyProvider = (): ProviderConfig => ({
  label: '', baseUrl: '', model: '', hasKey: false, verified: false, lastCheckedAt: null,
});
const defaultSettings = (): AppSettings => ({
  schemaVersion: 1, onboardingComplete: false, onboardingStep: 0,
  displayName: '', ambientMotion: true, assistantBrowserDrive: false, provider: emptyProvider(),
});
export const summaryOf = ({ id, title, createdAt, updatedAt, messageCount }: ChatWorkspace): WorkspaceSummary =>
  ({ id, title, createdAt, updatedAt, messageCount });

export class AppError extends Error {
  constructor(public readonly code: string, message: string) { super(message); }
}

export function friendlyError(error: unknown): string {
  const code = error && typeof error === 'object' && 'code' in error ? String(error.code).toLowerCase() : '';
  if (code.startsWith('runtime_')) {
    if (code === 'runtime_assets') return 'Bundled Hermes files are missing or invalid. Rebuild or reinstall Forma. Your chats are unchanged.';
    if (code === 'runtime_platform') return 'Managed Hermes is not supported on this platform yet. No fallback was used.';
    if (code === 'runtime_close') return 'Forma cannot confirm safe background operation. Keep this window open or explicitly Quit Forma.';
    if (code === 'runtime_conflict') return 'This runtime object changed, or work is still pending. Refresh it before retrying; your last valid view is unchanged.';
    if (code === 'runtime_not_found' || code === 'runtime_deleted') return 'This runtime object is unavailable or was deleted. Refresh the library.';
    if (code === 'runtime_auth') return 'Hermes denied this device or operation. Check its grants.';
    if (code === 'runtime_unresolved' || code === 'runtime_network') return 'The Hermes run is unresolved. Use Reconcile or Stop; sending again could create separate work.';
    if (code === 'runtime_not_ready') return 'Hermes is not ready. Check your model configuration or retry starting Hermes.';
    if (code === 'runtime_limit') return 'This runtime view exceeds the bounded item or data limit. It was not silently truncated.';
    if (code === 'runtime_binding_changed') return 'The managed Hermes profile identity changed. Existing work is preserved; retry or repair the app.';
    return 'The dedicated runtime returned an unsupported response. Your last valid work is preserved; no direct fallback was used.';
  }
  if (code.includes('corrupt') || code.includes('schema') || code.includes('storage_format')) {
    return 'Your saved work could not be opened. It has not been replaced. Try reopening Forma, or restore a backup.';
  }
  if (code.includes('not_found') || code.includes('workspace_missing')) return 'This workspace is no longer available.';
  if (code.includes('browser')) return 'Open Forma on desktop to set up AI. You can keep drafting here.';
  if (code.includes('unverified') || code.includes('not_ready') || code.includes('provider_missing')) return 'Set up and check your AI connection in Settings first.';
  if (code.includes('auth') || code.includes('credential') || code.includes('keychain')) return 'Your AI connection needs attention. Check its settings and try again.';
  if (code.includes('timeout') || code.includes('network') || code.includes('http')) return 'The connection did not finish. Your work is still here. Try again when you’re ready.';
  if (code === 'model_unavailable') return 'This model is no longer available. Refresh the list and choose another model.';
  if (code === 'models_empty') return 'This connection returned no models. Refresh the list or check Settings.';
  if (code === 'busy') return 'Wait for any reply, connection check, or model refresh to finish, then try again.';
  if (code === 'config_changed' || code === 'stale_request') return 'The connection changed. Refresh the models and try again.';
  if (code.includes('conflict')) return 'This draft changed in another window. Your text is still here; copy it before reopening this workspace.';
  if (code.includes('storage') || code.includes('database') || code.includes('persist')) return 'Your latest changes could not be saved. Keep this window open and try again.';
  if (code.includes('context') || code.includes('too_large') || code.includes('limit')) return 'This is a little too much for one request. Try a shorter message or a new chat.';
  if (code.includes('cancel')) return 'The reply was stopped.';
  // Never surface raw transport errors, remote response bodies, or private paths.
  return 'That didn’t finish. Your work is still here. Please try again.';
}

interface BrowserState { schemaVersion: 1; settings: AppSettings; workspaces: ChatWorkspace[] }
const record = (value: unknown): value is Record<string, unknown> => !!value && typeof value === 'object' && !Array.isArray(value);
const text = (value: unknown, max: number): value is string => typeof value === 'string' && value.length <= max;
const date = (value: unknown): value is string => text(value, 64) && Number.isFinite(Date.parse(value));
const integer = (value: unknown): value is number => Number.isSafeInteger(value) && Number(value) >= 0;
const exactKeys = (value: Record<string, unknown>, keys: string[]) => Object.keys(value).every(key => keys.includes(key));
function validMessage(value: unknown): value is ChatMessage {
  return record(value) && exactKeys(value, ['id', 'role', 'content', 'status', 'createdAt', 'requestId']) &&
    text(value.id, 100) && !!value.id && ['user', 'assistant'].includes(String(value.role)) &&
    text(value.content, 100_000) && ['complete', 'pending', 'error', 'cancelled'].includes(String(value.status)) &&
    date(value.createdAt) && (value.requestId === null || text(value.requestId, 100));
}
function validWorkspace(value: unknown): value is ChatWorkspace {
  return record(value) && exactKeys(value, ['id', 'title', 'createdAt', 'updatedAt', 'messageCount', 'draft', 'draftVersion', 'messages']) &&
    text(value.id, 100) && !!value.id && text(value.title, 200) && date(value.createdAt) && date(value.updatedAt) &&
    integer(value.messageCount) && text(value.draft, 32_000) && integer(value.draftVersion) &&
    Array.isArray(value.messages) && value.messages.every(validMessage) && value.messageCount === value.messages.length;
}
function validSettings(value: unknown): value is AppSettings {
  if (!record(value) || !exactKeys(value, ['schemaVersion', 'onboardingComplete', 'onboardingStep', 'displayName', 'ambientMotion', 'assistantBrowserDrive', 'provider'])) return false;
  const provider = value.provider;
  return value.schemaVersion === 1 && typeof value.onboardingComplete === 'boolean' &&
    integer(value.onboardingStep) && value.onboardingStep <= 2 && text(value.displayName, 80) &&
    typeof value.ambientMotion === 'boolean' && typeof value.assistantBrowserDrive === 'boolean' && record(provider) &&
    exactKeys(provider, ['label', 'baseUrl', 'model', 'hasKey', 'verified', 'lastCheckedAt']) &&
    provider.label === '' && provider.baseUrl === '' && provider.model === '' && provider.hasKey === false &&
    provider.verified === false && provider.lastCheckedAt === null;
}

/** Browser mode stores only nonsecret local work. Native failure never falls back here. */
export class BrowserBridge implements AppBridge {
  readonly native = false;
  constructor(private readonly storage: Storage = window.localStorage) {}

  private read(): BrowserState {
    let raw: string | null;
    try { raw = this.storage.getItem(BROWSER_STORAGE_KEY); }
    catch { throw new AppError('storage_unavailable', 'Local storage is unavailable.'); }
    if (raw === null) return { schemaVersion: 1, settings: defaultSettings(), workspaces: [] };
    try {
      const parsed: unknown = JSON.parse(raw);
      if (!record(parsed) || !exactKeys(parsed, ['schemaVersion', 'settings', 'workspaces']) || parsed.schemaVersion !== 1 ||
          !validSettings(parsed.settings) || !Array.isArray(parsed.workspaces) || !parsed.workspaces.every(validWorkspace) ||
          new Set(parsed.workspaces.map(workspace => workspace.id)).size !== parsed.workspaces.length) throw new Error();
      return parsed as unknown as BrowserState;
    } catch { throw new AppError('storage_format', 'Saved data was not changed.'); }
  }

  private write(state: BrowserState) {
    try { this.storage.setItem(BROWSER_STORAGE_KEY, JSON.stringify(state)); }
    catch { throw new AppError('storage_write', 'Local storage could not save changes.'); }
  }

  private mutate(id: string, change: (workspace: ChatWorkspace) => void): ChatWorkspace {
    const state = this.read();
    const workspace = state.workspaces.find(item => item.id === id);
    if (!workspace) throw new AppError('not_found', 'Workspace not found.');
    change(workspace);
    this.write(state);
    return structuredClone(workspace);
  }

  async bootstrap(): Promise<Bootstrap> {
    const state = this.read();
    return { settings: state.settings, workspaces: state.workspaces.map(summaryOf).sort((a, b) => b.updatedAt.localeCompare(a.updatedAt)) };
  }
  async saveSettings(input: SettingsInput) {
    const state = this.read();
    const next = { ...input, schemaVersion: 1 as const, provider: emptyProvider() };
    if (!validSettings(next)) throw new AppError('invalid_settings', 'Invalid settings.');
    state.settings = next;
    this.write(state);
    return structuredClone(next);
  }
  async configureProvider(_input: ProviderInput): Promise<ProviderConfig> { throw new AppError('browser_unsupported', 'Desktop only.'); }
  async checkProvider(): Promise<never> { throw new AppError('browser_unsupported', 'Desktop only.'); }
  async listModels(): Promise<never> { throw new AppError('browser_unsupported', 'Desktop only.'); }
  async createWorkspace() {
    const state = this.read();
    const now = new Date().toISOString();
    const workspace: ChatWorkspace = {
      id: crypto.randomUUID(), title: 'New chat', createdAt: now, updatedAt: now,
      messageCount: 0, draft: '', draftVersion: 0, messages: [],
    };
    state.workspaces.unshift(workspace);
    this.write(state);
    return structuredClone(workspace);
  }
  async getWorkspace(id: string) {
    const workspace = this.read().workspaces.find(item => item.id === id);
    if (!workspace) throw new AppError('not_found', 'Workspace not found.');
    return structuredClone(workspace);
  }
  async updateDraft(id: string, draft: string, version: number) {
    if (!text(draft, 32_000) || !integer(version)) throw new AppError('input_limit', 'Invalid draft.');
    return this.mutate(id, workspace => {
      if (version <= workspace.draftVersion) throw new AppError('draft_conflict', 'Draft version is stale.');
      workspace.draft = draft;
      workspace.draftVersion = version;
      workspace.updatedAt = new Date().toISOString();
    });
  }
  async renameWorkspace(id: string, title: string) {
    if (!title.trim() || title.trim().length > 200) throw new AppError('input_limit', 'Invalid title.');
    return this.mutate(id, workspace => { workspace.title = title.trim(); workspace.updatedAt = new Date().toISOString(); });
  }
  async deleteWorkspace(id: string) {
    const state = this.read();
    state.workspaces = state.workspaces.filter(workspace => workspace.id !== id);
    this.write(state);
  }
  async startMessage(): Promise<never> { throw new AppError('browser_unsupported', 'Desktop only.'); }
  async completeMessage(): Promise<never> { throw new AppError('browser_unsupported', 'Desktop only.'); }
  async cancelMessage(): Promise<never> { throw new AppError('browser_unsupported', 'Desktop only.'); }
}

export class NativeBridge implements AppBridge {
  readonly native = true;
  runtimeStatus = () => invoke<import('./runtime-contracts').RuntimeStatus>('runtime_status');
  runtimeWorkspace = (workspaceId: string) => invoke<import('./runtime-contracts').RuntimeWorkspace>('runtime_workspace', { workspaceId });
  runtimeRetry = () => invoke<import('./runtime-contracts').RuntimeStatus>('runtime_retry');
  finishWindowClose = () => invoke<'hidden' | 'closed'>('finish_window_close');
  runtimeSelectModel = (workspaceId: string, model: string) => invoke<import('./runtime-contracts').RuntimeWorkspace>('runtime_select_model', { workspaceId, model });
  runtimeCheck = () => invoke<import('./runtime-contracts').RuntimeStatus>('runtime_check');
  runtimeProgress = (workspaceId: string, requestId: string) => invoke<import('./runtime-contracts').RuntimeProgress>('runtime_progress', { workspaceId, requestId });
  bootstrap = () => invoke<Bootstrap>('app_bootstrap');
  saveSettings = (input: SettingsInput) => invoke<AppSettings>('save_settings', { input });
  configureProvider = (input: ProviderInput) => invoke<ProviderConfig>('configure_provider', { input });
  checkProvider = () => invoke<Awaited<ReturnType<AppBridge['checkProvider']>>>('check_provider');
  listModels = () => invoke<Awaited<ReturnType<AppBridge['listModels']>>>('list_models');
  createWorkspace = () => invoke<ChatWorkspace>('create_workspace');
  getWorkspace = (workspaceId: string) => invoke<ChatWorkspace>('get_workspace', { workspaceId });
  updateDraft = (workspaceId: string, draft: string, version: number) => invoke<ChatWorkspace>('update_draft', { workspaceId, draft, version });
  renameWorkspace = (workspaceId: string, title: string) => invoke<ChatWorkspace>('rename_workspace', { workspaceId, title });
  deleteWorkspace = (workspaceId: string) => invoke<void>('delete_workspace', { workspaceId });
  startMessage = (workspaceId: string, content: string, requestId: string) => invoke<ChatWorkspace>('start_message', { workspaceId, content, requestId });
  completeMessage = (workspaceId: string, requestId: string) => invoke<ChatWorkspace>('complete_message', { workspaceId, requestId });
  cancelMessage = (workspaceId: string, requestId: string) => invoke<ChatWorkspace>('cancel_message', { workspaceId, requestId });
}

export function createBridge(): AppBridge { return isTauri() ? new NativeBridge() : new BrowserBridge(); }
