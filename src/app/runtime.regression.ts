import { AppStore } from './store.ts';
import { ControlledBridge } from './store.regression.ts';
import { AppError } from './bridge.ts';
import { observeRuntimeAction, runtimeBridge } from './runtime.ts';
import type { RuntimeStatus, RuntimeWorkspace } from './runtime-contracts';

function assert(value: unknown, message: string): asserts value { if (!value) throw new Error(message); }
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(yes => { resolve = yes; }); return { promise, resolve }; }
async function until(condition: () => boolean) { for (let n = 0; n < 100; n++) { if (condition()) return; await Promise.resolve(); } throw new Error('Expected runtime test state was not reached.'); }
const copy = <T,>(value: T): T => structuredClone(value);
class RuntimeFixture extends ControlledBridge {
  runtime: RuntimeStatus = {
    state: 'ready', message: null, verified: true, generation: 1,
    models: [{ id: 'hermes-one', name: 'Hermes one', available: true }, { id: 'hermes-two', name: 'Hermes two', available: true }],
    capabilities: { contractVersion: 'forma-runtime-v1', deviceId: crypto.randomUUID(), libraryId: crypto.randomUUID(), modelOrigin: 'https://model.example',
      runtime: { kind: 'hermes', ready: true, revision: '349e6611a1c5d846a865368dd6c386b78edd1a54' },
      features: { eventPolling: true, interfaces: true, schedules: true, nativeToolBridge: true, generatedCodeExecution: false, liveGoogle: false }, tools: [],
      limits: { maxIterations: 8, maxToolCalls: 12, maxOutputTokens: 4096, maxDurationSeconds: 180, maxToolResultBytes: 262144 } },
  };
  selections = new Map<string, RuntimeWorkspace>();
  modelGate?: ReturnType<typeof deferred<void>>;
  statusGate?: ReturnType<typeof deferred<RuntimeStatus>>;
  modelCalls: { id: string; model: string }[] = [];
  runtimeStatus = async () => this.statusGate ? this.statusGate.promise : copy(this.runtime);
  runtimeWorkspace = async (workspaceId: string) => {
    if (!this.selections.has(workspaceId)) this.selections.set(workspaceId, { workspaceId, route: 'hermes', modelId: 'hermes-one', generation: 1, remoteInitialized: false });
    return copy(this.selections.get(workspaceId)!);
  };
  runtimeSelectModel = async (id: string, model: string) => {
    this.modelCalls.push({ id, model }); if (this.modelGate) await this.modelGate.promise;
    const workspace = await this.runtimeWorkspace(id); workspace.modelId = model; workspace.generation++; this.selections.set(id, workspace); return copy(workspace);
  };
  runtimeCheck = async () => copy(this.runtime);
  runtimeRetry = async () => copy(this.runtime);
}
async function setup() { const bridge = new RuntimeFixture(); const store = new AppStore(bridge); await store.initialise(); assert(store.getSnapshot().ready, 'Runtime fixture must hydrate.'); return { bridge, store }; }

export async function runRuntimeRegressionChecks(): Promise<string[]> {
  const passed: string[] = [];
  {
    const { bridge, store } = await setup();
    assert(bridge.startCalls === 0 && bridge.checkCalls === 0 && bridge.configureInputs.length === 0, 'Hydration never checks models, reads keys or configures a customer runtime.');
    for (const method of ['configure', 'disable', 'setRoute']) assert(!(method in runtimeBridge), `No manual runtime API: ${method}`);
    for (const field of ['endpoint', 'authToken', 'hasToken', 'enabled', 'allowTailscaleHttp']) assert(!(field in store.getSnapshot().runtime!), `No customer runtime setting: ${field}`);
    let retries = 0; bridge.runtimeRetry = async () => { retries++; return copy(bridge.runtime); };
    assert(await store.retryRuntime() && retries === 1, 'Retry manages Hermes without endpoint or token parameters.');
    passed.push('managed Hermes bootstrap and Retry require no customer runtime configuration or model calls');
  }
  {
    const { bridge, store } = await setup();
    bridge.settings.provider.verified = true;
    await store.saveSettings({ displayName: 'Runtime fixture' });
    store.editDraft('Hermes request'); const sending = store.send(); await until(() => bridge.completeCalls === 1);
    bridge.completion.resolve(copy(bridge.completedSnapshot!)); await sending;
    assert(bridge.configureInputs.length === 0 && bridge.checkCalls === 0, 'Hermes must not configure or check direct provider.');
    assert(store.getSnapshot().workspace?.messages[1].status === 'complete', 'Hermes uses existing durable chat lifecycle.');
    passed.push('managed Hermes uses the approved model without a direct execution route');
  }
  {
    const { bridge, store } = await setup();
    Object.defineProperty(bridge, 'runtimeWorkspace', { value: undefined });
    store.editDraft('No legacy fallback');
    let blocked = false; try { await store.send(); } catch { blocked = true; }
    assert(blocked && bridge.startCalls === 0 && store.getSnapshot().draft === 'No legacy fallback', 'A legacy bridge without managed Hermes cannot send directly.');
    await store.flush();
    passed.push('legacy native bridges cannot use direct execution fallback');
  }
  {
    const { bridge, store } = await setup(); bridge.runtime.state = 'error'; await store.refreshRuntime(); store.editDraft('Keep draft');
    let failed = false; try { await store.send(); } catch { failed = true; }
    assert(failed && bridge.startCalls === 0 && store.getSnapshot().draft === 'Keep draft', 'Disabled Hermes cannot fall back to verified direct provider.');
    passed.push('disabled Hermes retains draft and never silently uses direct');
  }
  {
    const { bridge, store } = await setup(); const a = store.getSnapshot().activeId!; await store.newWorkspace(); const b = store.getSnapshot().activeId!;
    await store.selectWorkspace(a); store.editDraft('Unchanged workspace draft'); await store.flush();
    bridge.modelGate = deferred<void>(); const selection = store.selectModel('hermes-two'); await until(() => bridge.modelCalls.length === 1);
    await store.selectWorkspace(b); bridge.modelGate.resolve(); await selection;
    assert(store.getSnapshot().runtimeWorkspace?.workspaceId === b && store.getSnapshot().runtimeWorkspace?.modelId === 'hermes-one', 'Late selection from A must not relabel B.');
    await store.selectWorkspace(a); assert(store.getSnapshot().runtimeWorkspace?.modelId === 'hermes-two', 'A remembers accepted model.');
    assert(store.getSnapshot().draft === 'Unchanged workspace draft' && bridge.configureInputs.length === 0, 'Model selection must preserve drafts and direct config.');
    passed.push('accepted model selection is workspace-scoped across navigation');
  }
  {
    const { bridge, store } = await setup(); store.editDraft('Runtime in flight'); const sending = store.send(); await until(() => bridge.completeCalls === 1);
    const configuring = store.configureProvider({ label: 'Rotated provider', baseUrl: 'https://model-new.example', model: 'model-two' });
    await until(() => bridge.cancelCalls === 1);
    bridge.completion.resolve(copy(bridge.completedSnapshot!)); await sending; await configuring;
    assert(store.getSnapshot().workspace?.messages[1].status === 'cancelled', 'Provider changes fence Hermes results using the prior model key.');
    passed.push('provider rotation cancels old Hermes work and rejects late output');
  }
  {
    const { bridge, store } = await setup(); const old = deferred<RuntimeStatus>(); bridge.statusGate = old; const earlier = store.refreshRuntime();
    bridge.statusGate = undefined; bridge.runtime.generation = 2; await store.refreshRuntime(); old.resolve({ ...copy(bridge.runtime), generation: 1 }); await earlier;
    assert(store.getSnapshot().runtime?.generation === 2, 'An older runtime refresh must not overwrite current endpoint generation.');
    passed.push('out-of-order configuration observations are generation fenced');
  }
  {
    const { bridge, store } = await setup(); const id = store.getSnapshot().activeId!; const host = bridge.workspaces.get(id)!;
    const requestId = crypto.randomUUID(); host.messages.push({ id: crypto.randomUUID(), role: 'assistant', content: '', status: 'pending', createdAt: new Date().toISOString(), requestId }); host.messageCount = 1;
    const reopened = new AppStore(bridge); await reopened.initialise(); reopened.editDraft('Do not duplicate');
    let failed = false; try { await reopened.send(); } catch (error) { failed = error instanceof AppError; }
    assert(failed && bridge.startCalls === 0, 'Restarted unresolved run blocks fresh admission.');
    const resuming = reopened.resumeRuntimeReply(); await until(() => bridge.completeCalls === 1); bridge.completion.resolve(copy(bridge.completedSnapshot!)); await resuming;
    assert(bridge.startCalls === 0, 'Reconciliation must reuse complete lifecycle, never start a new UUID.');
    passed.push('restart reconciliation does not duplicate an uncertain run');
  }
  {
    const { bridge } = await setup();
    const snapshots: (RuntimeStatus | null)[] = [];
    const failure = new AppError('runtime_binding_changed', 'Changed model destination.');
    let caught: unknown;
    try {
      await observeRuntimeAction(async () => { bridge.runtime.verified = false; throw failure; }, bridge.runtimeStatus, status => snapshots.push(status));
    } catch (error) { caught = error; }
    assert(caught === failure && snapshots[0] === null && snapshots.at(-1)?.verified === false, 'Failed check must invalidate then reload native binding state while preserving its error.');
    snapshots.length = 0;
    try { await observeRuntimeAction(async () => { throw failure; }, async () => { throw new Error('Host unavailable'); }, status => snapshots.push(status)); } catch { /* Expected. */ }
    assert(snapshots.at(-1) === null, 'Failure to refresh must leave no cached approval status.');
    passed.push('runtime hook action refreshes failed binding checks and fails closed if status read fails');
  }
  {
    const { bridge, store } = await setup();
    bridge.runtimeCheck = async () => { bridge.runtime.verified = false; throw new AppError('runtime_binding_changed', 'Changed destination.'); };
    let failed = false; try { await store.refreshModels(); } catch { failed = true; }
    assert(failed && store.getSnapshot().runtime?.verified === false, 'AppStore must not retain its independent stale binding snapshot after failed Check.');
    store.editDraft('Preserved after binding change');
    try { await store.send(); } catch { /* Expected. */ }
    assert(bridge.startCalls === 0 && store.getSnapshot().draft === 'Preserved after binding change', 'No admission under a failed current binding.');
    bridge.runtimeStatus = async () => { throw new Error('Host unavailable'); };
    try { await store.refreshModels(); } catch { /* Expected. */ }
    assert(store.getSnapshot().runtime === null, 'Unavailable native status cannot retain stale checked status.');
    await store.flush();
    passed.push('failed model checks refresh the AppStore snapshot and preserve drafts without admission');
  }
  {
    const { bridge, store } = await setup();
    bridge.runtime.capabilities!.runtime.ready = false; bridge.runtime.state = 'error'; await store.refreshRuntime(); store.editDraft('Worker is down');
    let failed = false; try { await store.send(); } catch { failed = true; }
    assert(failed && store.getSnapshot().runtime?.verified === true && bridge.startCalls === 0, 'Verified binding alone is not execution readiness.');
    assert(store.getSnapshot().draft === 'Worker is down', 'Worker unavailability must preserve the draft.');
    await store.flush();
    passed.push('binding verification and executor readiness remain separate in the chat store');
  }
  {
    const { bridge, store } = await setup(); const id = store.getSnapshot().activeId!; const host = bridge.workspaces.get(id)!;
    const requestId = crypto.randomUUID();
    host.messages.push({ id: crypto.randomUUID(), role: 'assistant', content: '', status: 'pending', createdAt: new Date().toISOString(), requestId }); host.messageCount = 1;
    const attempts: string[] = []; let failedOnce = false; const terminal = deferred<typeof host>();
    bridge.completeMessage = async (_workspaceId: string, resumedId?: string) => {
      attempts.push(resumedId ?? 'missing');
      if (!failedOnce) { failedOnce = true; throw new AppError('runtime_network', 'Cancellation still uncertain.'); }
      return terminal.promise;
    };
    const reopened = new AppStore(bridge); await reopened.initialise(); reopened.editDraft('Next only after stop');
    await reopened.resumeRuntimeReply();
    let blocked = false; try { await reopened.send(); } catch { blocked = true; }
    assert(blocked && bridge.startCalls === 0 && reopened.getSnapshot().workspace?.messages[0].status === 'pending', 'Failed recovered cancellation cannot free a workspace for fresh work.');
    const retry = reopened.resumeRuntimeReply(); await until(() => attempts.length === 2);
    assert(reopened.getSnapshot().pending && attempts.every(value => value === requestId), 'Durable cancellation retries reconcile only the original UUID.');
    const stopped = copy(host); stopped.messages[0].status = 'cancelled'; terminal.resolve(stopped); await retry;
    assert(!reopened.getSnapshot().pending && reopened.getSnapshot().workspace?.messages[0].status === 'cancelled' && bridge.startCalls === 0, 'Only terminal cancellation releases recovered pending work.');
    assert(reopened.getSnapshot().draft === 'Next only after stop', 'Reconciliation cannot consume the next draft.');
    await reopened.flush();
    passed.push('recovered cancellation keeps pending work fenced across failures and retries the same UUID until terminal');
  }
  {
    const {bridge,store}=await setup(); bridge.runtime.state='needsModel'; bridge.runtime.capabilities!.runtime.ready=false; await store.refreshRuntime();
    const original=bridge.startMessage; bridge.startMessage=async (...args)=>{const result=await original(...args); bridge.runtime.state='ready'; bridge.runtime.capabilities!.runtime.ready=true; return result;};
    store.editDraft('Cold first Send'); const sending=store.send(); await until(()=>bridge.completeCalls===1);
    assert(store.getSnapshot().runtime?.state==='ready','First Send must publish native readiness after private model sync.');
    bridge.completion.resolve(copy(bridge.completedSnapshot!)); await sending;
    bridge.completeMessage=async()=>{bridge.runtime.state='error'; throw new AppError('runtime_network','Controller stopped');};
    store.editDraft('Controller crash'); await store.send();
    assert(store.getSnapshot().runtime?.state==='error','A failed run refreshes managed status and exposes Retry.');
    passed.push('first Send and controller failures refresh authoritative lifecycle status');
  }
  {
    const {bridge,store}=await setup(); bridge.runtime.state='needsModel'; bridge.runtime.models=[]; await store.refreshRuntime();
    const before=copy(bridge.settings.provider); await store.selectModel('hermes-two');
    assert(bridge.configureInputs.length===0 && JSON.stringify(bridge.settings.provider)===JSON.stringify(before),'Cold workspace picker must not rotate the app-wide provider or default model.');
    assert(store.getSnapshot().runtimeWorkspace?.modelId==='hermes-two','Only the acknowledged workspace model changes.');
    passed.push('cold workspace model selection never mutates global provider defaults or epochs');
  }
  {
    const {bridge,store}=await setup(); const stopped=deferred<void>();
    bridge.completeMessage=async()=>{bridge.completeCalls++; await stopped.promise; throw new AppError('runtime_network','Owned process stopped for Quit');};
    bridge.cancelMessage=async(id)=>{bridge.cancelCalls++; const host=bridge.workspaces.get(id)!; host.messages[1].status='error'; host.messages[1].content='Reply interrupted when Forma quit; not rolled back.'; bridge.runtime.state='error'; stopped.resolve(); return copy(host);};
    store.editDraft('Quit while pending'); const sending=store.send(); await until(()=>bridge.completeCalls===1); store.editDraft('Preserve next draft');
    await store.prepareClose(); await sending;
    assert(store.getSnapshot().closing && !store.getSnapshot().anyReplyPending,'Explicit Quit can finish after native local interruption with a dead controller.');
    assert(bridge.workspaces.get(store.getSnapshot().activeId!)!.draft==='Preserve next draft','Quit must still flush drafts.');
    assert(store.getSnapshot().workspace?.messages[1].status==='error','Quit is interruption, not a fabricated cancelled remote run.');
    passed.push('Quit with a stopped controller completes after durable draft flush and explicit local interruption');
  }
  return passed;
}
