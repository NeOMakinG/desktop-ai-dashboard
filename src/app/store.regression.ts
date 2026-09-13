import type { AppBridge, AppSettings, ChatWorkspace, ProviderCheck, ProviderInput, SettingsInput } from './contracts';
import { AppError, BrowserBridge, BROWSER_STORAGE_KEY, summaryOf } from './bridge.ts';
import { AppStore } from './store.ts';

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
async function until(condition: () => boolean) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (condition()) return;
    await Promise.resolve();
  }
  throw new Error('Expected state was not reached.');
}
class MemoryStorage implements Storage {
  private values = new Map<string, string>();
  get length() { return this.values.size; }
  getItem(key: string) { return this.values.get(key) ?? null; }
  setItem(key: string, value: string) { this.values.set(key, value); }
  removeItem(key: string) { this.values.delete(key); }
  clear() { this.values.clear(); }
  key(index: number) { return [...this.values.keys()][index] ?? null; }
}
const clone = <T,>(value: T): T => structuredClone(value);
const timestamp = () => new Date().toISOString();
const settings = (): AppSettings => ({
  schemaVersion: 1, onboardingComplete: true, onboardingStep: 2, displayName: '', ambientMotion: false, assistantBrowserDrive: false,
  provider: { label: 'Test connection', baseUrl: 'http://localhost:9999/v1', model: 'test-model', hasKey: false, verified: true, lastCheckedAt: null },
});

/** In-memory transport only. It performs no provider, native, or network calls. */
export class ControlledBridge implements AppBridge {
  readonly native = true;
  settings = settings();
  runtimeStatus = async (): Promise<import('./runtime-contracts').RuntimeStatus> => ({ state: 'ready', message: null, verified: true, generation: 1,
    models: this.models.map(id => ({ id, name: id, available: true })), capabilities: { contractVersion: 'forma-runtime-v1', deviceId: 'fixture-device', libraryId: 'fixture-library', modelOrigin: this.settings.provider.baseUrl,
    runtime: { kind: 'hermes', ready: true, revision: '349e6611a1c5d846a865368dd6c386b78edd1a54' }, features: { eventPolling: true, interfaces: true, schedules: true, nativeToolBridge: false, generatedCodeExecution: false, liveGoogle: false }, tools: [],
    limits: { maxIterations: 8, maxToolCalls: 12, maxOutputTokens: 4096, maxDurationSeconds: 180, maxToolResultBytes: 262144 } } });
  runtimeWorkspace = async (workspaceId: string): Promise<import('./runtime-contracts').RuntimeWorkspace> => ({ workspaceId, route: 'hermes', modelId: this.settings.provider.model, generation: 1, remoteInitialized: false });

  workspaces = new Map<string, ChatWorkspace>();
  completion = deferred<ChatWorkspace>();
  cancellation = deferred<ChatWorkspace>();
  startGate: ReturnType<typeof deferred<void>> | null = null;
  draftGate: ReturnType<typeof deferred<void>> | null = null;
  delayedCancel = false;
  failCheck = false;
  failDiscovery = false;
  failConfigure = false;
  models = ['test-model', 'claude-opus-5'];
  discoveryCalls = 0;
  discoveryGate: ReturnType<typeof deferred<{ models: string[] }>> | null = null;
  configureGate: ReturnType<typeof deferred<void>> | null = null;
  configureInputs: ProviderInput[] = [];
  checkCalls = 0;
  listModels = async () => {
    this.discoveryCalls += 1;
    if (this.discoveryGate) return this.discoveryGate.promise;
    if (this.failDiscovery) throw new AppError('network', 'Test discovery failure.');
    return { models: clone(this.models) };
  };
  failDraft = false;
  completeCalls = 0;
  startCalls = 0;
  cancelCalls = 0;
  draftCalls = 0;
  completedSnapshot?: ChatWorkspace;
  cancelledSnapshot?: ChatWorkspace;
  bootstrap = async () => ({ settings: clone(this.settings), workspaces: [...this.workspaces.values()].map(summaryOf) });
  saveSettings = async (input: SettingsInput) => (this.settings = { ...this.settings, ...input });
  configureProvider = async (input: ProviderInput) => {
    this.configureInputs.push(clone(input));
    if (this.configureGate) await this.configureGate.promise;
    if (this.failConfigure) throw new AppError('storage', 'Test save failure.');
    this.settings.provider = { ...this.settings.provider, label: input.label, baseUrl: input.baseUrl, model: input.model, verified: false };
    return clone(this.settings.provider);
  };
  checkProvider = async (): Promise<ProviderCheck> => {
    this.checkCalls += 1;
    if (this.failCheck) { this.settings.provider.verified = false; throw new AppError('network_error', 'Test failure.'); }
    if (!this.models.includes(this.settings.provider.model)) throw new AppError('model_unavailable', 'Test unavailable model.');
    this.settings.provider.verified = true;
    return { provider: clone(this.settings.provider), models: clone(this.models) };
  };
  createWorkspace = async () => {
    const workspace: ChatWorkspace = { id: crypto.randomUUID(), title: 'New chat', createdAt: timestamp(), updatedAt: timestamp(), messageCount: 0, draft: '', draftVersion: 0, messages: [] };
    this.workspaces.set(workspace.id, workspace);
    return clone(workspace);
  };
  getWorkspace = async (id: string) => clone(this.require(id));
  private require(id: string) {
    const workspace = this.workspaces.get(id);
    if (!workspace) throw new AppError('not_found', 'Missing workspace.');
    return workspace;
  }
  updateDraft = async (id: string, draft: string, version: number) => {
    this.draftCalls += 1;
    if (this.draftGate) await this.draftGate.promise;
    if (this.failDraft) throw new AppError('storage_write', 'Test failure.');
    const workspace = this.require(id);
    assert(version > workspace.draftVersion, 'Draft updates must be strictly newer.');
    workspace.draft = draft; workspace.draftVersion = version; workspace.updatedAt = timestamp();
    return clone(workspace);
  };
  renameWorkspace = async (id: string, title: string) => { const workspace = this.require(id); workspace.title = title; return clone(workspace); };
  deleteWorkspace = async (id: string) => { this.workspaces.delete(id); };
  startMessage = async (id: string, content: string, requestId: string) => {
    this.startCalls += 1;
    const workspace = this.require(id);
    workspace.messages.push({ id: crypto.randomUUID(), role: 'user', content, status: 'complete', createdAt: timestamp(), requestId });
    workspace.messages.push({ id: crypto.randomUUID(), role: 'assistant', content: '', status: 'pending', createdAt: timestamp(), requestId });
    workspace.messageCount = workspace.messages.length;
    workspace.draft = ''; workspace.draftVersion += 1;
    const snapshot = clone(workspace);
    if (this.startGate) await this.startGate.promise;
    return snapshot;
  };
  completeMessage = async (id: string) => {
    this.completeCalls += 1;
    const workspace = this.require(id);
    workspace.messages[workspace.messages.length - 1].status = 'complete';
    workspace.messages[workspace.messages.length - 1].content = 'A controlled test response.';
    this.completedSnapshot = clone(workspace);
    return this.completion.promise;
  };
  cancelMessage = async (id: string) => {
    this.cancelCalls += 1;
    const workspace = this.require(id);
    workspace.messages[workspace.messages.length - 1].status = 'cancelled';
    workspace.messages[workspace.messages.length - 1].content = '';
    this.cancelledSnapshot = clone(workspace);
    return this.delayedCancel ? this.cancellation.promise : this.cancelledSnapshot;
  };
}

async function createStore() {
  const bridge = new ControlledBridge();
  const store = new AppStore(bridge);
  await store.initialise();
  assert(store.getSnapshot().ready, 'Store must hydrate before edits.');
  return { bridge, store };
}

/** Callable from a test runner; not imported by the production entrypoint. */
export async function runStoreRegressionChecks(): Promise<string[]> {
  const passed: string[] = [];
  {
    const { bridge, store } = await createStore();
    store.editDraft('First prompt');
    const sending = store.send();
    await until(() => bridge.completeCalls === 1);
    const older = clone(bridge.completedSnapshot!);
    store.editDraft('A newer, already saved draft');
    await store.flush();
    assert(!store.getSnapshot().dirty, 'New draft must be acknowledged before the delayed response.');
    bridge.completion.resolve(older);
    await sending;
    assert(store.getSnapshot().draft === 'A newer, already saved draft', 'Late completion must not clear an acknowledged newer draft.');
    assert(store.getSnapshot().workspace?.draftVersion === bridge.workspaces.get(store.getSnapshot().activeId!)?.draftVersion, 'Draft version must remain current.');
    passed.push('late completion preserves acknowledged newer draft');
  }
  {
    const { bridge, store } = await createStore();
    bridge.delayedCancel = true;
    store.editDraft('First prompt');
    const sending = store.send();
    await until(() => bridge.completeCalls === 1);
    store.cancel();
    await until(() => bridge.cancelCalls === 1);
    store.editDraft('Keep this newer draft too');
    await store.flush();
    bridge.completion.resolve(bridge.completedSnapshot!);
    await Promise.resolve(); await Promise.resolve();
    assert(store.getSnapshot().pending, 'A pending cancellation must retain the request slot.');
    await store.send();
    assert(bridge.startCalls === 1, 'A new request must not start before cancellation acknowledgement.');
    bridge.cancellation.resolve(bridge.cancelledSnapshot!);
    await sending;
    assert(store.getSnapshot().draft === 'Keep this newer draft too', 'Late cancellation must not clear an acknowledged newer draft.');
    assert(!store.getSnapshot().pending, 'Successful cancellation must release the request slot.');
    passed.push('late cancellation preserves draft and blocks overlapping send');
  }
  {
    const { bridge, store } = await createStore();
    bridge.startGate = deferred<void>();
    store.editDraft('Cancel during start');
    const sending = store.send();
    await until(() => bridge.startCalls === 1);
    store.cancel();
    assert(bridge.cancelCalls === 0, 'Cancel must wait for the start acknowledgement.');
    bridge.startGate.resolve();
    await sending;
    assert(Number(bridge.cancelCalls) === 1 && bridge.completeCalls === 0, 'Cancelled-before-ack requests must cancel once and never complete.');
    passed.push('cancel waits for start acknowledgement');
  }
  {
    const { bridge, store } = await createStore();
    bridge.failCheck = true;
    await store.checkProvider().catch(() => {});
    assert(store.getSnapshot().settings?.provider.verified === false, 'A failed check must not leave a verified badge.');
    passed.push('failed connection check clears readiness');
  }
  {
    const { bridge, store } = await createStore();
    const id = store.getSnapshot().activeId!;
    bridge.draftGate = deferred<void>();
    store.editDraft('Draft in an issued write');
    const saving = store.flush();
    await until(() => bridge.draftCalls === 1);
    store.editDraft('Draft queued behind that write');
    const deleting = store.deleteWorkspace(id);
    bridge.draftGate.resolve();
    await Promise.all([saving, deleting]);
    await store.flush();
    assert(!bridge.workspaces.has(id), 'Deleted workspace must stay absent from storage.');
    assert(!store.getSnapshot().workspaces.some(workspace => workspace.id === id), 'Deleted workspace must stay absent from history.');
    assert(bridge.draftCalls === 1, 'Queued draft must never write after deletion is requested.');
    passed.push('delete drains issued draft and discards queued draft without resurrection');
  }
  {
    const { bridge, store } = await createStore();
    store.editDraft('Recoverable unsaved work');
    bridge.failDraft = true;
    await store.prepareClose().catch(() => {});
    assert(!store.getSnapshot().closing && store.getSnapshot().dirty, 'A failed close flush must retain the editable draft.');
    bridge.failDraft = false;
    await store.prepareClose();
    assert(!store.getSnapshot().dirty && store.getSnapshot().closing, 'A successful close retry must acknowledge the draft first.');
    passed.push('close flush failure preserves recoverable draft');
  }
  {
    const storage = new MemoryStorage();
    const first = new BrowserBridge(storage);
    const workspace = await first.createWorkspace();
    await first.updateDraft(workspace.id, 'Remember me after reopening', 1);
    const reopened = new BrowserBridge(storage);
    assert((await reopened.getWorkspace(workspace.id)).draft === 'Remember me after reopening', 'Browser draft must survive bridge recreation.');
    await reopened.deleteWorkspace(workspace.id);
    await first.updateDraft(workspace.id, 'Stale writer', 2).then(() => { throw new Error('Deleted IDs must reject draft updates.'); }, () => {});
    assert((await reopened.bootstrap()).workspaces.length === 0, 'Stale browser writer must not resurrect deleted workspace.');
    await reopened.configureProvider({ label: '', baseUrl: '', model: '', apiKey: 'must-never-be-stored' }).catch(() => {});
    assert(!storage.getItem(BROWSER_STORAGE_KEY)?.includes('must-never-be-stored'), 'Browser provider secrets must never be persisted.');
    storage.setItem(BROWSER_STORAGE_KEY, '{invalid data');
    await reopened.createWorkspace().then(() => { throw new Error('Corrupt storage must reject mutation.'); }, () => {});
    assert(storage.getItem(BROWSER_STORAGE_KEY) === '{invalid data', 'Corrupt storage must not be silently reset.');
    passed.push('browser restart, deletion, secret denial and corrupt-storage preservation');
  }
  {
    const { bridge, store } = await createStore();
    assert(bridge.discoveryCalls === 0 && store.getSnapshot().models.length === 0, 'Hydration must not discover or invent models.');
    await store.refreshModels();
    assert(store.getSnapshot().settings?.provider.verified, 'Discovery must not invalidate verification.');
    bridge.failDiscovery = true;
    await store.refreshModels().catch(() => {});
    assert(store.getSnapshot().modelsStatus === 'error' && store.getSnapshot().models.includes('claude-opus-5'), 'Discovery errors must retain choices.');
    bridge.failDiscovery = false;
    bridge.models = [];
    await store.refreshModels();
    assert(store.getSnapshot().modelsStatus === 'ready' && store.getSnapshot().models.length === 0, 'Empty inventory must be explicit, not invented.');
    bridge.models = ['claude-opus-5'];
    await store.checkProvider().catch(() => {});
    await store.refreshModels();
    assert(!store.getSnapshot().settings?.provider.verified && store.getSnapshot().models[0] === 'claude-opus-5', 'Unavailable selected model must not prevent independent recovery discovery.');
    passed.push('explicit discovery, empty inventory, error retention and unavailable-model recovery');
  }
  {
    const { bridge, store } = await createStore();
    bridge.settings.provider.hasKey = true;
    await store.refreshModels();
    bridge.configureGate = deferred<void>();
    const selecting = store.selectModel('claude-opus-5');
    await until(() => bridge.configureInputs.length === 1);
    assert(bridge.checkCalls === 0 && store.getSnapshot().selectingModel, 'Check must wait for durable configure acknowledgement.');
    const input = bridge.configureInputs[0];
    assert(!('apiKey' in input) && !('clearKey' in input), 'Model selection must omit key fields, never overwrite stored credentials.');
    assert(input.baseUrl === bridge.settings.provider.baseUrl && input.label === bridge.settings.provider.label, 'Model selection must retain connection identity.');
    await store.selectModel('test-model').then(() => { throw new Error('Overlapping selection must reject.'); }, () => {});
    store.editDraft('Do not send during configuration');
    await store.send().catch(() => {});
    assert(bridge.startCalls === 0, 'Send must not cross model configuration.');
    await store.flush();
    bridge.configureGate.resolve();
    await selecting;
    assert(store.getSnapshot().settings?.provider.model === 'claude-opus-5' && store.getSnapshot().settings?.provider.verified, 'Selection is ready only after successful check.');
    assert(store.getSnapshot().settings?.provider.hasKey, 'Key must remain stored.');
    const reopened = new AppStore(bridge);
    await reopened.initialise();
    assert(reopened.getSnapshot().settings?.provider.model === 'claude-opus-5', 'Selection must survive store recreation.');
    bridge.failCheck = true;
    await store.selectModel('test-model').catch(() => {});
    assert(!store.getSnapshot().settings?.provider.verified && !!store.getSnapshot().modelSelectionError, 'Failed selection check must not claim success.');
    assert(store.getSnapshot().models.includes('claude-opus-5'), 'Failed check must leave recovery choices available.');
    bridge.failCheck = false;
    await store.selectModel('test-model');
    assert(store.getSnapshot().settings?.provider.verified && !store.getSnapshot().modelSelectionError, 'Same saved but unchecked model must be retryable.');
    bridge.failConfigure = true;
    const checks = bridge.checkCalls;
    await store.selectModel('claude-opus-5').catch(() => {});
    assert(bridge.checkCalls === checks && !store.getSnapshot().settings?.provider.verified, 'Failed persistence must not proceed to check or show success.');
    bridge.failConfigure = false;
    await store.checkProvider();
    const configurations = bridge.configureInputs.length;
    assert(store.getSnapshot().modelSelectionError, 'Regression requires a selection error remaining after an independent successful check.');
    await store.selectModel('test-model');
    assert(!store.getSnapshot().modelSelectionError && bridge.configureInputs.length === configurations, 'Choosing the verified current model must clear old selection errors without reconfiguring.');
    passed.push('serialized selection, key omission, persistence, failed-save/check recovery and busy-send guard');
  }
  {
    const { bridge, store } = await createStore();
    await store.refreshModels();
    store.editDraft('Active elsewhere');
    const sending = store.send();
    await until(() => bridge.completeCalls === 1);
    await store.newWorkspace();
    assert(!store.getSnapshot().pending && store.getSnapshot().anyReplyPending, 'Busy state must cover inactive workspaces.');
    await store.selectModel('claude-opus-5').then(() => { throw new Error('Active reply must block model changes.'); }, () => {});
    assert(bridge.configureInputs.length === 0 && bridge.cancelCalls === 0, 'Picker must not silently cancel a reply.');
    bridge.completion.resolve(bridge.completedSnapshot!);
    await sending;
    assert(!store.getSnapshot().anyReplyPending, 'Global busy state must recover after completion.');
    passed.push('model changes blocked by replies in any workspace');
  }
  {
    const { bridge, store } = await createStore();
    await store.refreshModels();
    bridge.discoveryGate = deferred<{ models: string[] }>();
    const stale = store.refreshModels();
    const previous = store.getSnapshot().settings!.provider;
    await store.configureProvider({ label: previous.label, baseUrl: 'http://localhost:9998/v1', model: previous.model });
    assert(store.getSnapshot().models.length === 0 && store.getSnapshot().modelsStatus === 'idle', 'Provider changes must clear the cached catalog.');
    const oldGate = bridge.discoveryGate;
    bridge.discoveryGate = null;
    bridge.models = ['new-connection-model'];
    await store.refreshModels();
    oldGate.resolve({ models: ['stale-connection-model'] });
    await stale;
    assert(store.getSnapshot().models[0] === 'new-connection-model', 'Late discovery must not overwrite the current provider catalog.');
    bridge.discoveryGate = deferred<{ models: string[] }>();
    const staleFailure = store.refreshModels();
    await store.configureProvider({ label: previous.label, baseUrl: previous.baseUrl, model: previous.model });
    bridge.discoveryGate.reject(new AppError('network', 'Stale failure'));
    await staleFailure;
    assert(store.getSnapshot().modelsError === null && store.getSnapshot().modelsStatus === 'idle', 'Stale discovery errors must not contaminate a new provider.');
    passed.push('provider-scoped cache and stale discovery success/error guards');
  }
  {
    const { bridge, store } = await createStore();
    bridge.discoveryGate = deferred<{ models: string[] }>();
    let secondSettled = false;
    const first = store.refreshModels();
    const second = store.refreshModels();
    void second.then(() => { secondSettled = true; });
    assert(first === second, 'Concurrent refresh calls must share the same pending promise.');
    await until(() => bridge.discoveryCalls === 1);
    assert(!secondSettled, 'A coalesced refresh await must not resolve before discovery.');
    bridge.discoveryGate.resolve({ models: ['test-model'] });
    await Promise.all([first, second]);
    assert(secondSettled && bridge.discoveryCalls === 1, 'One discovery must settle both refresh callers.');
    bridge.discoveryGate = deferred<{ models: string[] }>();
    const failedFirst = store.refreshModels();
    const failedSecond = store.refreshModels();
    const failures = Promise.allSettled([failedFirst, failedSecond]);
    await until(() => bridge.discoveryCalls === 2);
    bridge.discoveryGate.reject(new AppError('network', 'Shared discovery failure.'));
    assert((await failures).every(result => result.status === 'rejected'), 'Coalesced refresh failures must reject every caller.');
    bridge.discoveryGate = null;
    await store.refreshModels();
    assert(store.getSnapshot().modelsStatus === 'ready' && Number(bridge.discoveryCalls) === 3, 'A settled refresh promise must not prevent retry.');
    passed.push('coalesced refresh waits, shared failures and retry');
  }
  {
    for (const outcome of ['success', 'error']) {
      const { bridge, store } = await createStore();
      bridge.discoveryGate = deferred<{ models: string[] }>();
      const stale = store.refreshModels();
      await until(() => bridge.discoveryCalls === 1);
      bridge.models = ['test-model', 'newer-check-model'];
      await store.checkProvider();
      if (outcome === 'success') bridge.discoveryGate.resolve({ models: ['older-discovery-model'] });
      else bridge.discoveryGate.reject(new AppError('network', 'Older discovery failure.'));
      await stale;
      assert(store.getSnapshot().models.includes('newer-check-model') && !store.getSnapshot().models.includes('older-discovery-model'), 'Older discovery must not overwrite a newer successful check catalog.');
      assert(store.getSnapshot().modelsStatus === 'ready' && !store.getSnapshot().modelsError, 'Older discovery errors must not erase a newer successful check state.');
    }
    passed.push('newer successful checks supersede delayed discovery success and failure');
  }
  {
    const { bridge, store } = await createStore();
    await store.refreshModels();
    bridge.discoveryGate = deferred<{ models: string[] }>();
    const stale = store.refreshModels();
    await until(() => bridge.discoveryCalls === 2);
    bridge.failCheck = true;
    await store.checkProvider().catch(() => {});
    assert(store.getSnapshot().modelsStatus === 'error' && store.getSnapshot().modelsError, 'A failed newer check must settle catalog loading into a recoverable error.');
    assert(store.getSnapshot().models.includes('claude-opus-5'), 'Check failure must retain previously discovered choices.');
    bridge.discoveryGate.resolve({ models: ['older-discovery-model'] });
    await stale;
    assert(store.getSnapshot().modelsStatus === 'error' && !store.getSnapshot().models.includes('older-discovery-model'), 'Older discovery must not replace the failed newer check state.');
    bridge.discoveryGate = null;
    await store.refreshModels();
    assert(store.getSnapshot().modelsStatus === 'ready' && !store.getSnapshot().modelsError, 'A failed check must not strand the catalog loading lock.');
    passed.push('failed newer check settles loading, retains choices and allows refresh recovery');
  }
  {
    const browser = new BrowserBridge(new MemoryStorage());
    const store = new AppStore(browser);
    await store.initialise();
    await store.refreshModels().then(() => { throw new Error('Browser must deny discovery.'); }, () => {});
    await browser.listModels().then(() => { throw new Error('Browser bridge must deny model IPC.'); }, () => {});
    assert(store.getSnapshot().models.length === 0 && store.getSnapshot().modelsStatus === 'idle', 'Browser preview must not manufacture inventory.');
    passed.push('browser model discovery denial');
  }
  return passed;
}
