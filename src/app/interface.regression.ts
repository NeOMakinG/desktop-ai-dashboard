import { readFile } from 'node:fs/promises';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { transformWithOxc } from 'vite';
import { emptyInterfaceSpec, isInterfaceTimestamp, validateInterfaceSpec, type InterfaceSpec } from './interface-contracts.ts';
import { globalShortcutBlocked, LibraryRequestFence, nextLibraryRow, validateScheduleBounds } from './library-interactions.ts';
import type { RuntimeSchedule, RuntimeStatus } from './runtime-contracts.ts';

function assert(value: unknown, message: string): asserts value { if (!value) throw new Error(message); }

// First-party test sources only, transformed in memory. No app build/server or generated-code execution.
async function firstPartyModule(name: string): Promise<any> {
  const cache = new Map<string, Promise<string>>();
  const moduleUrl = (url: URL): Promise<string> => {
    const existing = cache.get(url.href); if (existing) return existing;
    const task = (async () => {
      let source = await readFile(url, 'utf8');
      if (url.pathname.endsWith('.json')) source = `export default ${source}`;
      else source = (await transformWithOxc(source, url.pathname, { jsx: { runtime: 'automatic' } })).code;
      const imports = [...source.matchAll(/from (['"])(.*?)\1/g)];
      for (const match of imports) {
        const specifier = match[2];
        let resolved: string;
        if (!specifier.startsWith('.')) resolved = import.meta.resolve(specifier);
        else {
          const path = new URL(specifier, url); let child = path;
          if (!/\.(tsx?|json)$/.test(path.pathname)) {
            try { await readFile(new URL(`${specifier}.ts`, url), 'utf8'); child = new URL(`${specifier}.ts`, url); }
            catch { child = new URL(`${specifier}.tsx`, url); }
          }
          resolved = await moduleUrl(child);
        }
        source = source.replace(match[0], `from ${JSON.stringify(resolved)}`);
      }
      return `data:text/javascript;base64,${Buffer.from(source).toString('base64')}`;
    })();
    cache.set(url.href, task); return task;
  };
  return import(await moduleUrl(new URL(name, import.meta.url)));
}
/** Exercises the production reload transaction and actual action markup with synthetic acknowledgements. */
async function checkScheduleReloadAndAvailability() {
  const { readScheduleSnapshot, ScheduleControls, SchedulesView } = await firstPartyModule('./SchedulesView.tsx');
  const { runtimeUnavailable, runtimeExecutionUnavailable, InterfacesView } = await firstPartyModule('./InterfacesView.tsx');
  const schedule: RuntimeSchedule = {
    id: 'schedule-1', workspaceId: 'workspace-1', interfaceId: 'interface-1', interfaceRevisionPolicy: 'latestAtRunStart', version: 4,
    state: 'enabled', prompt: 'Refresh synthetic conversation content', modelId: 'model-1', cron: '0 9 * * 1-5', timezone: 'UTC',
    endAt: '2099-09-17T18:00:00Z', maxRuns: 3, runsStarted: 0, grantRefs: [],
    budgets: { maxDurationSeconds: 120, maxToolCalls: 8, maxIterations: 6, maxOutputTokens: 2048 },
    createdAt: '2026-09-10T00:00:00Z', updatedAt: '2026-09-10T00:00:00Z', nextRunAt: '2099-09-11T09:00:00Z',
  };
  let authoritative = schedule; let detailCalls = 0;
  let detail: () => Promise<RuntimeSchedule> = async () => authoritative;
  const bridge = {
    listSchedules: async () => [authoritative], listInterfaces: async () => [], listRuns: async () => authoritative.lastRunId ? [{ id: authoritative.lastRunId, scheduleId: schedule.id, state: 'failed' }] : [],
    getSchedule: async (id: string) => { assert(id === schedule.id, 'Reload requests the selected stable schedule ID.'); detailCalls++; return detail(); },
  };
  const fence = new LibraryRequestFence();
  const reload = () => { const sequence = fence.issue(); return readScheduleSnapshot(schedule.workspaceId, schedule.id, () => fence.current(sequence), bridge); };
  const initial = await reload(); assert(initial.selected.runsStarted === 0 && !initial.selected.lastRunId, 'First acknowledgement accurately records never-run state.');
  authoritative = { ...schedule, runsStarted: 1, lastRunId: 'run-1', nextRunAt: '2099-09-12T09:00:00Z', reason: 'attempt_failed' };
  const attempted = await reload();
  assert(detailCalls === 2 && attempted.selected.version === initial.selected.version, 'Every acknowledged reload fetches selected detail even at the identical CAS version.');
  assert(attempted.selected.runsStarted === 1 && attempted.selected.lastRunId === 'run-1' && attempted.selected.nextRunAt === '2099-09-12T09:00:00Z' && attempted.selected.reason === 'attempt_failed' && attempted.runs[0].state === 'failed', 'Same-version attempt, next fire, last run, reason and actual history all refresh.');
  authoritative = { ...authoritative, state: 'ended', runsStarted: 3, nextRunAt: undefined, reason: 'attempt_limit_reached' };
  const ended = await reload();
  assert(ended.selected.version === schedule.version && ended.selected.state === 'ended' && ended.selected.runsStarted === 3 && !ended.selected.nextRunAt && ended.selected.reason === 'attempt_limit_reached', 'Same-version terminal execution acknowledgement replaces enabled detail.');
  let release!: (value: RuntimeSchedule) => void; let entered!: () => void;
  const waiting = new Promise<void>(resolve => { entered = resolve; });
  detail = () => { entered(); return new Promise(resolve => { release = resolve; }); };
  const older = reload(); await waiting; detail = async () => authoritative;
  const newer = await reload(); release(schedule);
  assert(await older === null && newer.selected.state === 'ended', 'An older detail response cannot overwrite a newer authoritative reload.');
  const waitingInvalidated = new Promise<void>(resolve => { entered = resolve; });
  detail = () => { entered(); return new Promise(resolve => { release = resolve; }); };
  const invalidated = reload(); await waitingInvalidated; fence.invalidate(); release(schedule);
  assert(await invalidated === null, 'Workspace/runtime change, selection or mutation invalidation discards in-flight schedule detail.');
  const removed = await readScheduleSnapshot(schedule.workspaceId, schedule.id, () => true, { ...bridge, listSchedules: async () => [] });
  assert(removed.selected === null, 'An acknowledged removed schedule clears selection rather than retaining stale controls.');
  const status: RuntimeStatus = { state: 'error', message: 'Executor unavailable', verified: true, generation: 1, models: [], capabilities: {
    contractVersion: 'forma-runtime-v1', deviceId: 'device-1', libraryId: 'library-1', modelOrigin: 'https://model.invalid',
    runtime: { kind: 'hermes', ready: false, revision: 'fixture', reason: 'executor_offline' },
    features: { eventPolling: true, interfaces: true, schedules: true, nativeToolBridge: false, generatedCodeExecution: false, liveGoogle: false }, tools: [], limits: { ...schedule.budgets, maxToolResultBytes: 1000 },
  } };
  const unavailable = runtimeUnavailable(true, status), executionReason = runtimeExecutionUnavailable(true, status);
  assert(unavailable === null && executionReason === 'executor_offline', 'Authenticated control-plane availability is independent of executor readiness.');
  const noop = () => {};
  const controls = (selected: RuntimeSchedule, blocked = unavailable, execution = executionReason) => renderToStaticMarkup(createElement(ScheduleControls, { selected, busy: false, unavailable: blocked, executionReason: execution, onPause: noop, onReview: noop, onCopy: noop }));
  const button = (markup: string, label: string) => { const match = [...markup.matchAll(/<button\b([^>]*)>(.*?)<\/button>/g)].find(match => match[2].replace(/<[^>]*>/g, '') === label); assert(match, `Action is rendered: ${label}`); return match[1]; };
  assert(!button(controls(schedule), 'Pause schedule').includes('disabled'), 'Actual Pause control stays enabled with authenticated binding and executor ready=false.');
  assert(button(controls({ ...schedule, state: 'paused' }), 'Review &amp; enable').includes('disabled'), 'Actual Enable review is blocked with executor ready=false.');
  assert(button(controls(ended.selected), 'Review &amp; enable').includes('disabled'), 'Reloaded terminal schedule cannot be enabled.');
  const ready = { ...status, state: 'ready' as const, capabilities: { ...status.capabilities!, runtime: { ...status.capabilities!.runtime, ready: true } } };
  assert(!button(controls({ ...schedule, state: 'paused' }, null, runtimeExecutionUnavailable(true, ready)), 'Review &amp; enable').includes('disabled'), 'Ready, verified, bounded paused schedule permits explicit enable review.');
  const invalidBinding = { ...ready, verified: false };
  assert(runtimeUnavailable(true, invalidBinding) && runtimeExecutionUnavailable(true, invalidBinding), 'Retained ready capability data cannot authorize an invalidated binding.');
  assert(button(controls({ ...schedule, state: 'paused' }, runtimeUnavailable(true, invalidBinding), runtimeExecutionUnavailable(true, invalidBinding)), 'Review &amp; enable').includes('disabled'), 'Binding invalidation blocks actual Enable review even when cached ready remains true.');
  const schedulesMarkup = renderToStaticMarkup(createElement(SchedulesView, { active: true, native: true, status, workspaceId: schedule.workspaceId, modelId: schedule.modelId, onSettings: noop }));
  assert(!button(schedulesMarkup, 'Reload status').includes('disabled'), 'Schedule listing remains accessible with the executor unavailable.');
  const interfacesMarkup = renderToStaticMarkup(createElement(InterfacesView, { active: true, native: true, status, workspaceId: schedule.workspaceId, onRevise: noop, onSettings: noop }));
  assert(!button(interfacesMarkup, 'Reload library').includes('disabled'), 'Interface listing is not disabled by worker unavailability.');
}

/** Startup polling only reads status, serializes work, and stops outside startup. */
async function checkManagedStartupPolling() {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const effect = source.match(/useLifecycle\(store\);\n(  useEffect\([\s\S]*?)\n  const focusComposer/);
  assert(effect, 'Managed startup status effect is present.');
  const compiled = (await transformWithOxc(effect[1], 'app-startup-fixture.ts')).code;
  const run = new Function('useEffect', 'store', 'state', 'setTimeout', 'clearTimeout', compiled);
  let next: (() => Promise<void>) | null = null; let cleanup: (() => void) | undefined; let calls = 0;
  let snapshot: any = { closing: false, runtime: { state: 'starting' } };
  let release!: () => void;
  const store = { bridge: { native: true }, getSnapshot: () => snapshot, refreshRuntime: () => { calls++; return new Promise<void>(resolve => { release = resolve; }); } };
  const timeout = (callback: () => Promise<void>, delay: number) => { assert(delay === 1000 && !next, 'Startup uses one serialized 1s status timer.'); next = callback; return 1; };
  const setup = () => run((callback: () => () => void) => { cleanup = callback(); }, store, snapshot, timeout, () => { next = null; });
  const fire = () => { const callback = next; assert(callback, 'Expected startup timer.'); next = null; callback(); };
  setup(); fire(); assert(calls === 1 && !next, 'No overlapping request while status read is pending.');
  release(); await Promise.resolve(); assert(next, 'Still-starting status schedules the next bounded read.');
  fire(); snapshot = { ...snapshot, runtime: { state: 'ready' } }; release(); await Promise.resolve(); assert(!next && Number(calls) === 2, 'Ready stops startup polling immediately.');
  cleanup?.();
  for (const state of ['ready', 'needsModel', 'error']) { snapshot = { closing: false, runtime: { state } }; setup(); assert(!next, 'Terminal managed state never starts a timer.'); }
  snapshot = { closing: false, runtime: { state: 'starting' } }; setup(); fire(); cleanup?.(); release(); await Promise.resolve(); assert(!next, 'Unmount cannot resurrect an in-flight startup timer.');
  snapshot = { closing: true, runtime: { state: 'starting' } }; setup(); assert(!next, 'Closing does not schedule more status work.');
}

/** Runs the actual first-party App handler with nonsecret events; never reads field values. */
async function checkNavigationShortcutGuards() {
  const source = await readFile(new URL('./App.tsx', import.meta.url), 'utf8');
  const handler = source.match(/const keydown = \(event: KeyboardEvent\) => \{[\s\S]*?\n    \};/);
  assert(handler, 'Production global keyboard handler remains regression-accessible.');
  const compiled = (await transformWithOxc(handler[0], 'app-shortcut-fixture.ts')).code;
  let effects = 0;
  const effect = () => { effects++; };
  const plain = { tagName: 'BUTTON', closest: () => null, isContentEditable: false };
  const scope: any = { activeElement: plain, querySelector: () => null };
  // Only checked-in first-party code is evaluated, in an offline test, not model-authored UI.
  const make = new Function('globalShortcutBlocked', 'document', 'settingsOpen', 'paletteOpen', 'shortcutsOpen', 'workspaceDialog', 'setPaletteOpen', 'setView', 'store', 'focusComposer', 'view', 'setShortcutsOpen', 'state', 'setSidebarOpen', `${compiled}\nreturn keydown;`);
  const create = (modal = false) => make(globalShortcutBlocked, scope, modal, false, false, null, effect, effect, { newWorkspace: () => { effect(); return Promise.resolve(); }, cancel: effect }, effect, 'chat', effect, { pending: true }, effect);
  const dispatch = (target: any, key: string, modifiers: Record<string, boolean> = {}, changes: Record<string, unknown> = {}, handler = create()) => {
    const event = { target, key, isComposing: false, defaultPrevented: false, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, composedPath: () => [target], preventDefault: effect, ...modifiers, ...changes };
    effects = 0; handler(event); return effects;
  };
  const shortcuts: [string, Record<string, boolean>][] = [['k', { metaKey: true }], ['k', { ctrlKey: true }], ['n', { metaKey: true, shiftKey: true }], ['n', { ctrlKey: true, shiftKey: true }], ['/', {}], ['?', {}], ['Escape', {}], ['j', {}], ['k', {}], ['ArrowDown', {}], ['Enter', {}]];
  const fields = [
    { ...plain, tagName: 'INPUT', type: 'password' }, { ...plain, tagName: 'INPUT', type: 'text' },
    { ...plain, tagName: 'INPUT', type: 'url' }, { ...plain, tagName: 'TEXTAREA' }, { ...plain, tagName: 'SELECT' },
    { ...plain, tagName: 'DIV', isContentEditable: true }, { ...plain, tagName: 'SPAN', closest: () => ({}) },
  ];
  for (const field of fields) for (const [key, modifiers] of shortcuts) {
    scope.activeElement = plain;
    assert(dispatch(field, key, modifiers) === 0, `Editable ${field.tagName} never triggers global navigation or cancellation.`);
    scope.activeElement = field;
    assert(dispatch(plain, key, modifiers) === 0, 'Focused editable protects retargeted native events.');
    scope.activeElement = plain;
    assert(dispatch(plain, key, modifiers, { composedPath: () => [field, plain] }) === 0, 'Editable shadow/event path blocks navigation.');
  }
  for (const [key, modifiers] of shortcuts) {
    assert(dispatch(plain, key, modifiers, { defaultPrevented: true }) === 0, 'Consumed component events never invoke global navigation.');
    assert(dispatch(plain, key, modifiers, { isComposing: true }) === 0, 'IME input remains untouched.');
    assert(dispatch(plain, key, modifiers, {}, create(true)) === 0, 'Settings-open state blocks shortcuts before native dialog effects.');
    scope.querySelector = () => ({});
    assert(dispatch(plain, key, modifiers) === 0, 'Open native or ARIA modal blocks navigation regardless of event target.');
    scope.querySelector = () => null;
  }
  assert(dispatch(plain, 'k', { metaKey: true }) > 0 && dispatch(plain, '/', {}) > 0, 'Normal noneditable shortcuts still work; guard is not blanket disabled.');
  const { listKeyboard } = await firstPartyModule('./InterfacesView.tsx');
  const previousDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
  Object.defineProperty(globalThis, 'document', { configurable: true, value: scope });
  try {
    for (const field of fields) for (const key of ['j', 'k', 'ArrowDown', 'ArrowUp']) {
      effects = 0;
      listKeyboard({ target: field, key, nativeEvent: { target: field, composedPath: () => [field] }, currentTarget: { querySelectorAll: () => { effect(); return []; } }, preventDefault: effect });
      assert(effects === 0, 'Actual library list handler leaves password/input/textarea/editable keys untouched.');
    }
  } finally { if (previousDocument) Object.defineProperty(globalThis, 'document', previousDocument); else Reflect.deleteProperty(globalThis, 'document'); }
  assert(source.includes('await store.prepareClose();') && source.includes('await store.bridge.finishWindowClose();') && source.includes("if (result === 'hidden') { store.resumeAfterClose(); closing = false; }") && !source.includes('window.destroy()'), 'Close flushes drafts before managed close and unlocks hidden-window reuse.');
}

/** Synthetic, offline schema and interaction checks. Not model generation or native runtime QA. */
export async function runInterfaceRegressionChecks(): Promise<string[]> {
  const base: InterfaceSpec = {
    schemaVersion: 1, kind: 'components',
    root: { id: 'root', type: 'grid', columns: 2, children: [
      { id: 'reading', type: 'card', title: '今週 · café / العربية', children: [{ id: 'intro', type: 'text', text: 'A novel composition, not a canned dashboard.' }, { id: 'trend', type: 'chart', kind: 'line', datasetId: 'series', xField: 'at', yField: 'value', label: 'Conversation trend', units: 'items' }] },
      { id: 'counts', type: 'stack', children: [{ id: 'value', type: 'metric', datasetId: 'headline', field: 'value', label: 'Explicit supplied count', format: 'number' }, { id: 'table', type: 'table', datasetId: 'series', columns: [{ field: 'at', label: 'UTC' }, { field: 'value', label: 'Count' }] }] },
    ] },
    datasets: [
      { id: 'headline', columns: [{ id: 'value', type: 'number' }], rows: [{ value: 0 }] },
      { id: 'series', columns: [{ id: 'at', type: 'timestamp' }, { id: 'value', type: 'number' }], rows: [{ at: '2026-09-10T00:00:00Z', value: -8 }, { at: '2026-09-11T00:00:00Z', value: null }, { at: '2026-09-12T00:00:00Z', value: 12 }] },
    ],
  };
  const valid = validateInterfaceSpec(base); assert(valid.ok, 'Novel nested Unicode composition must validate.');
  assert(validateInterfaceSpec(emptyInterfaceSpec()).ok, 'Empty draft must be valid and never fabricate data.');
  const reject = (change: (input: any) => void, reason: string) => { const value = structuredClone(base); change(value); assert(!validateInterfaceSpec(value).ok, reason); };
  reject(v => { v.schemaVersion = 2; }, 'Unknown schema version rejected.');
  reject(v => { v.kind = 'react'; }, 'Custom executable mode rejected.');
  reject(v => { v.sourceMode = 'live'; }, 'Model provenance authority rejected.');
  reject(v => { v.root.style = { color: 'red' }; }, 'Model style injection rejected.');
  reject(v => { v.root.onClick = 'invoke("shell")'; }, 'Model actions rejected.');
  reject(v => { v.root.children[0].children[0] = { id: 'script', type: 'html', html: '<script>1</script>' }; }, 'Executable HTML component rejected.');
  reject(v => { v.root.children[1].id = 'reading'; }, 'Duplicate node identity rejected.');
  reject(v => { v.datasets[0].id = 'constructor'; }, 'Prototype keys rejected.');
  reject(v => { v.datasets[1].columns.push({ id: 'value', type: 'number' }); }, 'Duplicate typed column rejected.');
  reject(v => { v.datasets[0].rows[0].value = Infinity; }, 'Infinite number rejected.');
  reject(v => { v.datasets[0].rows[0].value = NaN; }, 'NaN rejected.');
  reject(v => { v.datasets[0].rows[0].value = 1e12 + 1; }, 'Magnitude beyond1e12 rejected.');
  reject(v => { v.datasets[0].rows[0].value = '0'; }, 'No numeric coercion.');
  reject(v => { v.datasets[0].rows.push({ value: 3 }); }, 'Metric implicit aggregation rejected.');
  reject(v => { v.root.children[0].children[1].yField = 'at'; }, 'Chart numeric field enforced.');
  reject(v => { v.root.children[0].children[1].series = ['other']; }, 'Extra chart series rejected.');
  reject(v => { v.datasets[1].rows[1].at = '2026-09-10T00:00:00Z'; }, 'Duplicate line instant rejected.');
  reject(v => { v.datasets[1].rows.reverse(); }, 'Unordered line instants rejected.');
  reject(v => { v.datasets[1].rows[0].at = '2026-02-30T00:00:00Z'; }, 'Nonexistent calendar instant rejected.');
  reject(v => { v.datasets[1].rows[0].at = null; }, 'Missing x position rejected rather than interpolated.');
  reject(v => { v.datasets[1].rows[0].extra = 'account'; }, 'Unknown data column rejected.');
  reject(v => { delete v.datasets[1].rows[0].value; }, 'Missing declared field rejected.');
  reject(v => { v.root.children[0].children[0].text = 'é'.repeat(2001); }, 'Unicode character bound enforced.');
  reject(v => { v.root.children[0].children[0].text = '\u0000'; }, 'Control-character input rejected.');
  reject(v => { v.root.children[1].children[0].currency = 'USD'; }, 'Currency without currency format rejected.');
  reject(v => { v.root.children[1].children[0].format = 'currency'; v.root.children[1].children[0].currency = 'bad'; }, 'Invalid currency code rejected.');
  reject(v => { v.root = { id: 'r1', type: 'stack', children: [{ id: 'r2', type: 'stack', children: [{ id: 'r3', type: 'stack', children: [{ id: 'r4', type: 'stack', children: [] }] }] }] }; }, 'Fourth layout ancestor rejected.');
  reject(v => { v.root.children = Array.from({ length: 13 }, (_, i) => ({ id: `t${i}`, type: 'text', text: '' })); }, 'Content node limit enforced.');
  reject(v => { v.datasets[1].rows = Array.from({ length: 101 }, () => ({ at: '2026-09-10T00:00:00Z', value: 1 })); }, '101 data rows rejected.');
  reject(v => { v.root.columns = 5; }, 'Grid column ceiling enforced.');
  const cycle = emptyInterfaceSpec(); (cycle.root as any).children.push(cycle.root); assert(!validateInterfaceSpec(cycle).ok, 'Cycles rejected before serialization.');
  const payload = emptyInterfaceSpec(); payload.datasets = [{ id: 'large', columns: [{ id: 'text', type: 'text' }], rows: Array.from({ length: 100 }, () => ({ text: 'é'.repeat(1000) })) }]; assert(!validateInterfaceSpec(payload).ok, 'UTF8 whole payload cap enforced.');
  const chars = emptyInterfaceSpec(); chars.root = { id: 'unicode', type: 'text', text: '😀'.repeat(2000) }; assert(validateInterfaceSpec(chars).ok, 'Unicode surrogate pairs counted as characters.');
  const malformed = emptyInterfaceSpec(); malformed.root = { id: 'badUnicode', type: 'text', text: String.fromCharCode(0xd800) }; assert(!validateInterfaceSpec(malformed).ok, 'Lone surrogate rejected.');
  const literal = emptyInterfaceSpec(); literal.root = { id: 'literal', type: 'text', text: '<script>alert(1)</script> — conversation text' }; assert(validateInterfaceSpec(literal).ok, 'Literal angle brackets preserved for React text escaping, not evaluated.');
  const empty = structuredClone(base); empty.datasets.forEach(dataset => { dataset.rows = []; }); assert(validateInterfaceSpec(empty).ok, 'No supplied rows allowed without fake zero results.');
  assert(isInterfaceTimestamp('2026-09-10T12:00:00.123Z') && !isInterfaceTimestamp('2026-09-10T12:00:00+02:00'), 'UTC explicit timestamp contract.');
  const now = Date.parse('2026-09-10T00:00:00Z'), future = '2026-09-17T18:00:00Z';
  assert(validateScheduleBounds('0 9 * * 1-5', future, 10, now) === null, 'Finite five-field UTC schedule accepted.');
  for (const cron of ['* * * * * *', '@daily', '*/0 * * * *', '99 * * * *', '0 25 * * *', '0 0 * JAN *', '0 0 31-1 * *']) assert(validateScheduleBounds(cron, future, 10, now), `Invalid cron rejected: ${cron}`);
  for (const runs of [0, -1, Infinity, NaN, 101, 1.5]) assert(validateScheduleBounds('* * * * *', future, runs, now), 'Finite attempt cap enforced.');
  assert(validateScheduleBounds('* * * * *', '2026-09-09T00:00:00Z', 1, now), 'Past deadline rejected.');
  assert(nextLibraryRow('j', -1, 3, false) === 0 && nextLibraryRow('k', 0, 3, false) === 0 && nextLibraryRow('ArrowDown', 2, 3, false) === 2, 'Keyboard list movement bounded.');
  assert(nextLibraryRow('j', 0, 3, true) === null && nextLibraryRow('k', 0, 3, false, true) === null && nextLibraryRow('Enter', 0, 0, false) === null, 'Editing, modified shortcuts and empty lists untouched.');
  const fence = new LibraryRequestFence(); const load = fence.issue(); const newer = fence.issue(); assert(!fence.current(load) && fence.current(newer), 'New reads fence old responses.'); fence.invalidate(); assert(!fence.current(newer), 'Pause/enable/publish/delete invalidate earlier list/detail responses.');
  const { InterfaceRenderer } = await firstPartyModule('./InterfaceRenderer.tsx');
  const render = (spec: unknown) => renderToStaticMarkup(createElement(InterfaceRenderer, { spec }));
  const markup = render(base);
  for (const semantic of ['interface-grid', 'interface-card', 'interface-stack', 'interface-metric', '<svg', '<table', 'View data table', 'tabindex="0"', 'From conversation · not live']) assert(markup.includes(semantic), `Missing trusted renderer semantic: ${semantic}`);
  assert(!markup.includes('NaN') && !markup.includes('Infinity'), 'Finite chart geometry.');
  assert(render(literal).includes('&lt;script&gt;alert(1)&lt;/script&gt;') && !render(literal).includes('<script>'), 'Renderer escapes model HTML as literal text.');
  assert(render(emptyInterfaceSpec()).includes('Empty composition'), 'Empty composition renderer gives no invented metrics.');
  assert(render(empty).includes('No numeric points supplied') && render(empty).includes('No rows supplied'), 'Empty datasets distinguish missing points and source emptiness.');
  assert(render({ ...base, sourceMode: 'live' }).includes('role="alert"'), 'Unsafe provenance rejects whole preview with recoverable error.');
  const bars = structuredClone(base); bars.datasets[1].columns[0].type = 'text'; bars.datasets[1].rows.forEach((row, i) => { row.at = ['負', 'missing', 'positive'][i]; });
  const barNode = (bars.root as any).children[0].children[1]; barNode.kind = 'bar';
  assert(render(bars).includes('chart-bar') && render(bars).includes('負') && !render(bars).includes('NaN'), 'Negative and missing bar points have finite accessible labels.');
  const { toolActivity, managedHermesLabel, ManagedHermesStatus, hermesCanAcceptMessage } = await firstPartyModule('./RuntimeActivity.tsx');
  const requested = { runId: 'r', seq: 1, at: '2026-09-10T00:00:00Z', type: 'tool.requested', payload: { requestId: 't', toolName: 'forma_interface_propose' } };
  const result = { runId: 'r', seq: 2, at: '2026-09-10T00:00:01Z', type: 'tool.result', payload: { requestId: 't', outcome: 'succeeded' } };
  assert(toolActivity([requested, requested, result]).length === 1 && toolActivity([requested, result])[0].outcome === 'succeeded', 'Duplicate structured tool updates collapse to one invocation.');
  assert(toolActivity([{ runId: 'r', seq: 1, at: '', type: 'assistant.message', payload: { text: 'Tool succeeded!' } }]).length === 0, 'Assistant prose cannot fabricate tool events.');
  const managed = { state: 'ready', message: null, generation: 1, verified: true, models: [], capabilities: { runtime: { ready: true } } };
  assert(managedHermesLabel(true, null) === 'Hermes starting' && managedHermesLabel(true, managed) === 'Hermes ready', 'Managed readiness requires native acknowledgement.');
  assert(managedHermesLabel(true, { ...managed, state: 'needsModel' }) === 'Hermes needs a model' && managedHermesLabel(true, { ...managed, state: 'error' }) === 'Hermes error' && managedHermesLabel(true, { ...managed, verified: false }) === 'Hermes error' && managedHermesLabel(false, managed).includes('desktop app required'), 'Model/error/native-host states never claim readiness.');
  const needsModel = { ...managed, state: 'needsModel', capabilities: { runtime: { ready: false } } };
  assert(hermesCanAcceptMessage(true, needsModel, true, 'saved-model'), 'Explicit Send may attach the already checked provider to managed Hermes without a startup key read or extra setup.');
  for (const [native, status, provider, model] of [[false, needsModel, true, 'saved-model'], [true, needsModel, false, 'saved-model'], [true, needsModel, true, ''], [true, { ...needsModel, verified: false }, true, 'saved-model'], [true, { ...needsModel, state: 'error' }, true, 'saved-model'], [true, { ...needsModel, state: 'starting' }, true, 'saved-model']]) assert(!hermesCanAcceptMessage(native, status, provider, model), 'Unverified, model-less, starting, failed or nonnative execution stays blocked with no alternate route.');
  assert(hermesCanAcceptMessage(true, managed, true, 'saved-model') && !hermesCanAcceptMessage(true, { ...managed, capabilities: { runtime: { ready: false } } }, true, 'saved-model'), 'Ready still requires acknowledged executor readiness.');
  const statusMarkup = (status: unknown) => renderToStaticMarkup(createElement(ManagedHermesStatus, { state: { runtime: status }, store: { bridge: { native: true } }, onSettings: () => {} }));
  assert(statusMarkup({ ...managed, state: 'error' }).includes('>Retry</button>') && !statusMarkup(managed).includes('>Retry</button>') && statusMarkup({ ...managed, state: 'needsModel' }).includes('Model settings'), 'Managed errors offer Retry and missing models point only to provider/model settings.');
  assert(!statusMarkup(managed).includes('<select') && !statusMarkup(managed).includes('<input'), 'Managed status has no execution-route or credential controls.');
  const css = await readFile(new URL('./runtime-ui.css', import.meta.url), 'utf8');
  assert(!/#[0-9a-f]{3,8}\b/i.test(css), 'New surfaces use shared color tokens only.');
  const spacingScale = new Set([0, 2, 4, 8, 12, 16, 24, 32, 48, 64]);
  for (const declaration of css.matchAll(/(?:gap|padding(?:-[a-z]+)?|margin(?:-[a-z]+)?)\s*:\s*([^;}]+)/g)) for (const pixels of declaration[1].matchAll(/(-?\d+)px/g)) assert(spacingScale.has(Number(pixels[1])), `Off-scale new spacing: ${declaration[0]}`);
  assert(css.includes('.modal.library-modal') && css.includes('.library-page .library-search input'), 'Late-loading app.css cannot override new modal width or field spacing.');
  await checkScheduleReloadAndAvailability();
  const settingsSource = await readFile(new URL('./RuntimeSettings.tsx', import.meta.url), 'utf8');
  assert(!/<form\b|<input\b|authToken|allowTailscaleHttp|runtime\.configure|runtime\.disable|requestSubmit/.test(settingsSource), 'Runtime setup form and runtime credential handling are removed, not collapsed.');
  for (const file of ['App.tsx', 'Settings.tsx', 'RuntimeSettings.tsx', 'RuntimeActivity.tsx', 'ModelSelector.tsx', 'InterfacesView.tsx', 'SchedulesView.tsx']) {
    const source = await readFile(new URL(`./${file}`, import.meta.url), 'utf8');
    assert(!/Direct chat|Direct replies|Direct route|Workspace execution route|Configure runtime|runtimeSecret|runtime-token|Tailscale|\.setRoute\(/i.test(source), 'Customer UI contains no manual runtime route/configuration or credential surface.');
  }
  const scheduleSource = await readFile(new URL('./SchedulesView.tsx', import.meta.url), 'utf8');
  assert(scheduleSource.includes('Forma and this machine to remain running and awake') && scheduleSource.includes('Quitting Forma or powering off stops execution'), 'Schedules truthfully disclose app and machine lifetime.');
  await checkNavigationShortcutGuards();
  await checkManagedStartupPolling();
  const { ModelPicker } = await firstPartyModule('./ModelSelector.tsx');
  const recoveringModel = renderToStaticMarkup(createElement(ModelPicker, {
    state: { activeId: 'workspace-recovery', runtimeWorkspace: { workspaceId: 'workspace-recovery', modelId: 'saved-unavailable-model' }, runtime: { state: 'needsModel', verified: true, models: [] }, settings: { provider: { baseUrl: 'https://provider.invalid/v1', model: 'saved-unavailable-model', verified: true } }, models: ['discovered-recovery-model'], modelsStatus: 'ready' },
    store: { bridge: { native: true } }, onClose: () => {}, onSettings: () => {},
  }));
  assert(recoveringModel.includes('discovered-recovery-model') && recoveringModel.includes('role="option"') && recoveringModel.includes('Your saved model is unavailable'), 'Verified needsModel with an empty managed catalog retains selectable discovered-provider recovery choices.');
  const modelChecks = await firstPartyModule('./model-selector.regression.tsx');
  assert(modelChecks.runModelSelectorRegressionChecks().length >= 5, 'Existing explicit-choice model picker regressions preserved.');
  return ['novel flexible Unicode composition and empty states', 'strict schema, code/action/provenance denial', 'finite bounded typed data, metric and chart semantics', 'UTF8/rows/columns/depth/content limits', 'UTC finite schedule form guards', 'keyboard boundaries and stale-response fences', 'first-party SSR: chart/table, escaped HTML and recovery semantics', 'structured activity deduplication and managed Hermes states', 'existing model picker SSR regressions', 'new-surface token/spacing/specificity checks', 'same-version schedule execution reload and asynchronous response fences', 'authenticated unready executor: listing/Pause available, Enable blocked', 'managed-only UI and accurate desktop scheduler lifetime', 'production keyboard handler: passwords/fields/modals/IME/consumed events never navigate', 'managed close preserves draft flush and hidden-window recovery', 'managed startup polling serializes reads and stops at terminal state/close/unmount'];
}
