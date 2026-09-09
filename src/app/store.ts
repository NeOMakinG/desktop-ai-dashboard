import type { AppBridge, AppSettings, ChatWorkspace, ProviderInput, SettingsInput, WorkspaceSummary } from './contracts';
import { AppError, friendlyError, summaryOf } from './bridge.ts';

const ACTIVE_KEY = 'forma.active-workspace.v1';
interface ActiveRequest {
  id: string;
  generation: number;
  acked: boolean;
  cancelled: boolean;
  cancelTask?: Promise<void>;
  cancelFailed?: boolean;
  finished?: boolean;
  done?: Promise<void>;
}
interface Entry {
  data: ChatWorkspace;
  draft: string;
  version: number;
  dirty: boolean;
  saving: boolean;
  blocked: boolean;
  draftError: string | null;
  replyError: string | null;
  write: Promise<void>;
  timer?: ReturnType<typeof setTimeout>;
  maxTimer?: ReturnType<typeof setTimeout>;
  request?: ActiveRequest;
}
export interface AppSnapshot {
  ready: boolean;
  loading: boolean;
  error: string | null;
  settings: AppSettings | null;
  workspaces: WorkspaceSummary[];
  activeId: string | null;
  workspace: ChatWorkspace | null;
  draft: string;
  dirty: boolean;
  saving: boolean;
  draftError: string | null;
  replyError: string | null;
  pending: boolean;
  starting: boolean;
  navigating: boolean;
  closing: boolean;
  anyReplyPending: boolean;
  providerBusy: boolean;
  selectingModel: boolean;
  models: string[];
  modelsStatus: 'idle' | 'loading' | 'ready' | 'error';
  modelsError: string | null;
  modelSelectionError: string | null;
}

/** Owns durable-work ordering independently of component mounting or selection. */
export class AppStore {
  private entries = new Map<string, Entry>();
  private deleted = new Set<string>();
  private listeners = new Set<() => void>();
  private initialising?: Promise<void>;
  private settingsQueue: Promise<unknown> = Promise.resolve();
  private settingsPending = 0;
  private providerTasks = new Set<Promise<unknown>>();
  private providerRevision = 0;
  private generation = 0;
  private catalogRevision = 0;
  private modelRefresh?: Promise<void>;
  private snapshot: AppSnapshot = {
    ready: false, loading: false, error: null, settings: null, workspaces: [], activeId: null,
    workspace: null, draft: '', dirty: false, saving: false, draftError: null, replyError: null,
    pending: false, starting: false, navigating: false, closing: false,
    anyReplyPending: false, providerBusy: false, selectingModel: false,
    models: [], modelsStatus: 'idle', modelsError: null, modelSelectionError: null,
  };
  constructor(readonly bridge: AppBridge) {}
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  getSnapshot = () => this.snapshot;

  private emit(patch: Partial<AppSnapshot> = {}) {
    this.snapshot = { ...this.snapshot, ...patch };
    const entry = this.snapshot.activeId ? this.entries.get(this.snapshot.activeId) : undefined;
    this.snapshot = {
      ...this.snapshot, workspace: entry?.data ?? null, draft: entry?.draft ?? '',
      dirty: entry?.dirty ?? false, saving: entry?.saving ?? false,
      draftError: entry?.draftError ?? null, replyError: entry?.replyError ?? null,
      pending: !!entry?.request, starting: !!entry?.request && !entry.request.acked,
      anyReplyPending: [...this.entries.values()].some(item => !!item.request),
    };
    this.listeners.forEach(listener => listener());
  }
  private rememberActive(id: string | null) {
    try { if (id) localStorage.setItem(ACTIVE_KEY, id); else localStorage.removeItem(ACTIVE_KEY); } catch { /* Optional navigation preference, never chat data. */ }
  }
  private entryFrom(data: ChatWorkspace): Entry {
    return { data, draft: data.draft, version: data.draftVersion, dirty: false, saving: false,
      blocked: false, draftError: null, replyError: null, write: Promise.resolve() };
  }
  private publishSummary(data: ChatWorkspace) {
    if (this.deleted.has(data.id)) return;
    const workspaces = [summaryOf(data), ...this.snapshot.workspaces.filter(item => item.id !== data.id)]
      .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
    this.emit({ workspaces });
  }
  private applyMessages(entry: Entry, data: ChatWorkspace) {
    if (entry.blocked || this.deleted.has(data.id)) return;
    // Response snapshots can arrive after a newer draft has already been acknowledged.
    // Clean means durable, not old: only an equal/newer host version can replace it.
    if (!entry.dirty && data.draftVersion >= entry.version) entry.draft = data.draft;
    entry.version = Math.max(entry.version, data.draftVersion);
    entry.data = {
      ...data, draft: entry.draft, draftVersion: entry.version,
      title: entry.data.updatedAt > data.updatedAt ? entry.data.title : data.title,
      updatedAt: entry.data.updatedAt > data.updatedAt ? entry.data.updatedAt : data.updatedAt,
    };
    this.publishSummary(entry.data);
  }

  initialise = (): Promise<void> => {
    if (this.initialising) return this.initialising;
    this.initialising = this.load().finally(() => { this.initialising = undefined; });
    return this.initialising;
  };
  private async load() {
    if (this.snapshot.ready) return;
    this.emit({ loading: true, error: null });
    try {
      const bootstrap = await this.bridge.bootstrap();
      let preferred: string | null = null;
      try { preferred = localStorage.getItem(ACTIVE_KEY); } catch { /* Optional preference. */ }
      const id = bootstrap.workspaces.find(item => item.id === preferred)?.id ?? bootstrap.workspaces[0]?.id;
      const workspace = id ? await this.bridge.getWorkspace(id) : await this.bridge.createWorkspace();
      this.entries.set(workspace.id, this.entryFrom(workspace));
      this.rememberActive(workspace.id);
      this.emit({ ready: true, loading: false, settings: bootstrap.settings,
        workspaces: id ? bootstrap.workspaces : [summaryOf(workspace)], activeId: workspace.id });
    } catch (error) { this.emit({ loading: false, error: friendlyError(error) }); }
  }

  editDraft = (draft: string) => {
    const entry = this.snapshot.activeId ? this.entries.get(this.snapshot.activeId) : undefined;
    if (!entry || entry.blocked || this.snapshot.navigating || this.snapshot.closing || (entry.request && !entry.request.acked)) return;
    if (draft.length > 32_000 || draft === entry.draft) return;
    entry.draft = draft;
    entry.version += 1;
    entry.dirty = true;
    entry.draftError = null;
    clearTimeout(entry.timer);
    entry.timer = setTimeout(() => { void this.flushEntry(entry).catch(() => {}); }, 500);
    entry.maxTimer ??= setTimeout(() => { void this.flushEntry(entry).catch(() => {}); }, 2_000);
    this.emit();
  };
  private clearTimers(entry: Entry) {
    clearTimeout(entry.timer); clearTimeout(entry.maxTimer);
    entry.timer = undefined; entry.maxTimer = undefined;
  }
  private flushEntry(entry: Entry): Promise<void> {
    this.clearTimers(entry);
    const task = entry.write.catch(() => {}).then(async () => {
      while (entry.dirty && !entry.blocked && !this.deleted.has(entry.data.id)) {
        const draft = entry.draft;
        const version = entry.version;
        entry.saving = true;
        this.emit();
        try {
          const saved = await this.bridge.updateDraft(entry.data.id, draft, version);
          if (entry.blocked || this.deleted.has(entry.data.id)) return;
          if (saved.draftVersion !== version || saved.draft !== draft) throw new AppError('draft_conflict', 'Draft changed elsewhere.');
          // A draft acknowledgement must not overwrite newer messages or a renamed title.
          entry.data = { ...entry.data, draft: saved.draft, draftVersion: saved.draftVersion,
            updatedAt: saved.updatedAt > entry.data.updatedAt ? saved.updatedAt : entry.data.updatedAt };
          if (entry.version === version) entry.dirty = false;
          entry.draftError = null;
          this.publishSummary(entry.data);
        } catch (error) {
          if (!entry.blocked && !this.deleted.has(entry.data.id)) entry.draftError = friendlyError(error);
          throw error;
        } finally { entry.saving = false; this.emit(); }
      }
    });
    entry.write = task;
    return task;
  }
  flush = async () => {
    const failures = await Promise.allSettled([...this.entries.values()].map(entry => this.flushEntry(entry)));
    const failure = failures.find(result => result.status === 'rejected');
    if (failure?.status === 'rejected') throw failure.reason;
  };
  retryDraft = async () => {
    const entry = this.snapshot.activeId ? this.entries.get(this.snapshot.activeId) : undefined;
    if (entry) await this.flushEntry(entry);
  };

  selectWorkspace = async (id: string) => {
    if (id === this.snapshot.activeId || this.snapshot.navigating || this.snapshot.closing || this.deleted.has(id)) return;
    this.emit({ navigating: true, error: null });
    try {
      await this.flush();
      if (!this.entries.has(id)) this.entries.set(id, this.entryFrom(await this.bridge.getWorkspace(id)));
      if (this.deleted.has(id)) return;
      this.rememberActive(id);
      this.emit({ activeId: id });
    } catch (error) { this.emit({ error: friendlyError(error) }); throw error; }
    finally { this.emit({ navigating: false }); }
  };
  newWorkspace = async () => {
    if (this.snapshot.navigating || this.snapshot.closing) return;
    this.emit({ navigating: true, error: null });
    try {
      await this.flush();
      const workspace = await this.bridge.createWorkspace();
      this.entries.set(workspace.id, this.entryFrom(workspace));
      this.rememberActive(workspace.id);
      this.emit({ activeId: workspace.id });
      this.publishSummary(workspace);
    } catch (error) { this.emit({ error: friendlyError(error) }); throw error; }
    finally { this.emit({ navigating: false }); }
  };
  renameWorkspace = async (id: string, title: string) => {
    const entry = this.entries.get(id);
    if (entry?.blocked || this.deleted.has(id)) return;
    const saved = await this.bridge.renameWorkspace(id, title);
    if (this.deleted.has(id) || entry?.blocked) return;
    if (entry) entry.data = { ...entry.data, title: saved.title, updatedAt: saved.updatedAt };
    this.publishSummary(entry?.data ?? saved);
  };
  deleteWorkspace = async (id: string) => {
    if (this.snapshot.navigating || this.snapshot.closing || this.deleted.has(id)) return;
    const entry = this.entries.get(id);
    this.emit({ navigating: true, error: null });
    if (entry) { entry.blocked = true; this.clearTimers(entry); }
    try {
      // Drain already issued writes. Queued writes see blocked and never run.
      if (entry) {
        this.cancelEntry(entry);
        await entry.write.catch(() => {});
        await entry.request?.done?.catch(() => {});
      }
      await this.bridge.deleteWorkspace(id);
      this.deleted.add(id);
      this.entries.delete(id);
      const remaining = this.snapshot.workspaces.filter(item => item.id !== id);
      this.emit({ workspaces: remaining, activeId: this.snapshot.activeId === id ? null : this.snapshot.activeId });
      if (!this.snapshot.activeId) {
        const next = remaining[0];
        const workspace = next ? (this.entries.get(next.id)?.data ?? await this.bridge.getWorkspace(next.id)) : await this.bridge.createWorkspace();
        if (!this.entries.has(workspace.id)) this.entries.set(workspace.id, this.entryFrom(workspace));
        this.rememberActive(workspace.id);
        this.emit({ activeId: workspace.id });
        this.publishSummary(this.entries.get(workspace.id)!.data);
      }
    } catch (error) {
      if (entry && !this.deleted.has(id)) entry.blocked = false;
      this.emit({ error: friendlyError(error) });
      throw error;
    } finally { this.emit({ navigating: false }); }
  };

  send = async () => {
    const entry = this.snapshot.activeId ? this.entries.get(this.snapshot.activeId) : undefined;
    if (!entry || entry.blocked || entry.request || this.snapshot.navigating || this.snapshot.closing || !entry.draft.trim()) return;
    if (this.snapshot.providerBusy || this.snapshot.selectingModel) throw new AppError('busy', 'Connection is changing.');
    if (!this.bridge.native || !this.snapshot.settings?.provider.verified) throw new AppError('provider_not_ready', 'Set up AI first.');
    const request: ActiveRequest = { id: crypto.randomUUID(), generation: this.generation, acked: false, cancelled: false };
    entry.request = request;
    entry.replyError = null;
    this.emit();
    const task = this.runMessage(entry, request);
    request.done = task;
    await task;
  };
  private async runMessage(entry: Entry, request: ActiveRequest) {
    try {
      await this.flushEntry(entry);
      if (request.cancelled || entry.blocked) return;
      const started = await this.bridge.startMessage(entry.data.id, entry.draft.trim(), request.id);
      request.acked = true;
      if (!entry.blocked && !this.deleted.has(entry.data.id)) {
        entry.draft = started.draft;
        entry.version = started.draftVersion;
        entry.dirty = false;
        this.applyMessages(entry, started);
      }
      if (request.cancelled || request.generation !== this.generation || entry.blocked) {
        await this.cancelAcknowledged(entry, request);
        return;
      }
      const completed = await this.bridge.completeMessage(entry.data.id, request.id);
      if (!request.cancelled && request.generation === this.generation) this.applyMessages(entry, completed);
    } catch (error) {
      if (!entry.blocked && !this.deleted.has(entry.data.id) && !request.cancelled && request.generation === this.generation) {
        entry.replyError = friendlyError(error);
        if (request.acked) {
          try {
            const recovered = await this.bridge.getWorkspace(entry.data.id);
            if (!request.cancelled && request.generation === this.generation && entry.request === request) this.applyMessages(entry, recovered);
          } catch { /* Retain the last acknowledged history. */ }
        }
      }
    } finally {
      request.finished = true;
      // Do not release this workspace for a new send while an old cancel can still publish.
      if (request.cancelTask) await request.cancelTask.catch(() => {});
      if (entry.request === request && !request.cancelFailed) entry.request = undefined;
      this.emit();
    }
  }
  private cancelAcknowledged(entry: Entry, request: ActiveRequest): Promise<void> {
    request.cancelTask ??= this.bridge.cancelMessage(entry.data.id, request.id).then(saved => {
      request.cancelFailed = false;
      if (entry.request === request && !entry.blocked && !this.deleted.has(entry.data.id)) this.applyMessages(entry, saved);
    }).catch(error => {
      request.cancelFailed = true;
      if (!entry.blocked && !this.deleted.has(entry.data.id)) entry.replyError = 'The reply could not be stopped. Try Stop again before closing.';
      throw error;
    }).finally(() => {
      if (request.finished && !request.cancelFailed && entry.request === request) entry.request = undefined;
      this.emit();
    });
    return request.cancelTask;
  }
  private cancelEntry(entry: Entry) {
    const request = entry.request;
    if (!request) return;
    request.cancelled = true;
    if (request.cancelFailed) { request.cancelTask = undefined; request.cancelFailed = false; }
    if (request.acked) void this.cancelAcknowledged(entry, request).catch(() => {});
    this.emit();
  }
  cancel = () => {
    const entry = this.snapshot.activeId ? this.entries.get(this.snapshot.activeId) : undefined;
    if (entry) this.cancelEntry(entry);
  };
  private async cancelAll() {
    const requests = [...this.entries.values()].filter(entry => entry.request).map(entry => ({ entry, request: entry.request! }));
    requests.forEach(({ entry }) => this.cancelEntry(entry));
    await Promise.all(requests.map(async ({ request }) => {
      await request.done;
      if (request.cancelTask) await request.cancelTask;
    }));
  }
  private trackProvider<T>(task: Promise<T>): Promise<T> {
    this.providerTasks.add(task);
    return task.finally(() => { this.providerTasks.delete(task); });
  }
  private providerGuard() {
    if (!this.bridge.native) throw new AppError('browser_unsupported', 'Desktop only.');
    if (this.snapshot.closing) throw new AppError('closing', 'Window is closing.');
    if (this.snapshot.providerBusy || this.snapshot.selectingModel) throw new AppError('busy', 'Connection is changing.');
  }
  configureProvider = async (input: ProviderInput) => {
    this.providerGuard();
    return this.performConfigure(input);
  };
  private performConfigure = (input: ProviderInput) => {
    const generation = ++this.generation;
    this.providerRevision += 1;
    this.catalogRevision += 1;
    this.modelRefresh = undefined;
    const current = this.snapshot.settings?.provider;
    const sameConnection = current?.baseUrl === input.baseUrl && current.label === input.label && input.apiKey === undefined && !input.clearKey;
    // A model-only edit keeps recovery choices, but invalidates in-flight discovery.
    this.emit({ providerBusy: true, models: sameConnection ? this.snapshot.models : [],
      modelsStatus: sameConnection && this.snapshot.modelsStatus !== 'loading' ? this.snapshot.modelsStatus : 'idle',
      modelsError: null, modelSelectionError: null,
      ...(this.snapshot.settings ? { settings: { ...this.snapshot.settings, provider: { ...this.snapshot.settings.provider, verified: false } } } : {}) });
    return this.trackProvider((async () => {
      await this.cancelAll();
      const provider = await this.bridge.configureProvider(input);
      if (generation !== this.generation) throw new AppError('config_changed', 'Connection changed.');
      this.providerRevision += 1;
      if (this.snapshot.settings) this.emit({ settings: { ...this.snapshot.settings, provider } });
      return provider;
    })().finally(() => this.emit({ providerBusy: false })));
  };
  checkProvider = async () => {
    this.providerGuard();
    return this.performCheck();
  };
  private performCheck = () => {
    const generation = this.generation;
    // This newer inventory request owns the catalog, even when its check fails.
    const catalogRevision = ++this.catalogRevision;
    this.modelRefresh = undefined;
    this.providerRevision += 1;
    this.emit({ providerBusy: true, modelsStatus: 'loading', modelsError: null,
      ...(this.snapshot.settings ? { settings: { ...this.snapshot.settings, provider: { ...this.snapshot.settings.provider, verified: false } } } : {}) });
    return this.trackProvider((async () => {
      try {
        const result = await this.bridge.checkProvider();
        if (generation !== this.generation) throw new AppError('config_changed', 'Connection changed.');
        this.providerRevision += 1;
        if (this.snapshot.settings) this.emit({ settings: { ...this.snapshot.settings, provider: result.provider },
          ...(catalogRevision === this.catalogRevision ? { models: result.models, modelsStatus: 'ready' as const, modelsError: null } : {}) });
        return result;
      } catch (error) {
        if (generation === this.generation && catalogRevision === this.catalogRevision) this.emit({ modelsStatus: 'error', modelsError: friendlyError(error) });
        throw error;
      }
    })().finally(() => this.emit({ providerBusy: false })));
  };
  refreshModels = (): Promise<void> => {
    try { this.providerGuard(); } catch (error) { return Promise.reject(error); }
    if (this.modelRefresh) return this.modelRefresh;
    const generation = this.generation;
    const revision = ++this.catalogRevision;
    const task = this.trackProvider(Promise.resolve().then(async () => {
      try {
        const result = await this.bridge.listModels();
        if (generation !== this.generation || revision !== this.catalogRevision) return;
        this.emit({ models: result.models, modelsStatus: 'ready', modelsError: null });
      } catch (error) {
        if (generation !== this.generation || revision !== this.catalogRevision) return;
        this.emit({ modelsStatus: 'error', modelsError: friendlyError(error) });
        throw error;
      }
    })).finally(() => { if (this.modelRefresh === task) this.modelRefresh = undefined; });
    this.modelRefresh = task;
    this.emit({ modelsStatus: 'loading', modelsError: null });
    return task;
  };
  selectModel = async (model: string) => {
    this.providerGuard();
    if (this.snapshot.anyReplyPending) throw new AppError('busy', 'Finish active replies first.');
    const current = this.snapshot.settings?.provider;
    if (!current || !this.snapshot.models.includes(model)) throw new AppError('model_unavailable', 'Refresh available models first.');
    this.emit({ modelSelectionError: null });
    if (current.model === model && current.verified) return;
    this.emit({ selectingModel: true, modelSelectionError: null });
    return this.trackProvider((async () => {
      try {
        // Deliberately omit apiKey/clearKey: native retains the endpoint-bound key.
        await this.performConfigure({ label: current.label, baseUrl: current.baseUrl, model });
        const result = await this.performCheck();
        if (!result.provider.verified || result.provider.model !== model) throw new AppError('provider_not_ready', 'Selection was not verified.');
      } catch (error) {
        this.emit({ modelSelectionError: friendlyError(error) });
        throw error;
      } finally { this.emit({ selectingModel: false }); }
    })());
  };
  saveSettings = (patch: Partial<SettingsInput>): Promise<AppSettings> => {
    if (this.snapshot.closing) return Promise.reject(new AppError('closing', 'Window is closing.'));
    this.settingsPending += 1;
    const task = this.settingsQueue.catch(() => {}).then(async () => {
      const current = this.snapshot.settings;
      if (!current) throw new AppError('not_ready', 'Settings are not ready.');
      const providerRevision = this.providerRevision;
      const { onboardingComplete, onboardingStep, displayName, ambientMotion } = current;
      const saved = await this.bridge.saveSettings({ onboardingComplete, onboardingStep, displayName, ambientMotion, ...patch });
      const settings = providerRevision === this.providerRevision ? saved : { ...saved, provider: this.snapshot.settings!.provider };
      this.emit({ settings });
      return settings;
    });
    this.settingsQueue = task;
    return task.finally(() => { this.settingsPending -= 1; });
  };
  clearError = () => this.emit({ error: null });
  prepareClose = async () => {
    this.emit({ closing: true, error: null });
    try {
      await this.flush();
      if (this.settingsPending) await this.settingsQueue;
      await Promise.all([...this.providerTasks]);
      await this.cancelAll();
    } catch (error) {
      this.emit({ closing: false, error: friendlyError(error) });
      throw error;
    }
  };
  closeFailed = () => this.emit({ closing: false, error: 'This window could not close safely. Your work is still open.' });
}
