import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { BrowserBridge } from './bridge';
import { AppStore, type AppSnapshot } from './store';
import { filterModels, modelLabel, ModelPicker, ModelSelector } from './ModelSelector';

function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message);
}

/** Pure presentation regressions; no DOM, storage, native IPC, or provider requests. */
export function runModelSelectorRegressionChecks(): string[] {
  const store = new AppStore(new BrowserBridge({} as Storage));
  const nativeStore = Object.create(store) as AppStore;
  Object.defineProperty(nativeStore, 'bridge', { value: { native: true } });
  const base: AppSnapshot = {
    ...store.getSnapshot(),
    settings: { schemaVersion: 1, onboardingComplete: true, onboardingStep: 2, displayName: '', ambientMotion: false, assistantBrowserDrive: false,
      provider: { label: 'Fixture', baseUrl: 'http://localhost:9999/v1', model: 'claude-opus-5', hasKey: true, verified: true, lastCheckedAt: null } },
    models: ['astra', 'claude-opus-5', 'claude-opus-4-5-20251101'], modelsStatus: 'ready',
  };
  const render = (state: AppSnapshot, native = true) => renderToStaticMarkup(createElement(ModelPicker, {
    state, store: native ? nativeStore : store, onClose: () => {}, onSettings: () => {},
  }));
  assert(modelLabel('claude-opus-5') === 'Opus 5', 'Chip must humanize actual Opus ID.');
  assert(modelLabel('claude-opus-4-5-20251101') === 'Opus 4.5', 'Date suffix must not be a version label.');
  assert(modelLabel('claude-opus-4-20250514') === 'Opus 4', 'Older dated major-only ID must remain Opus 4.');
  assert(modelLabel('vendor/custom_model') === 'vendor/custom_model', 'Unknown IDs must stay exact.');
  assert(filterModels(base.models, ' OPUS 4.5 ')[0] === 'claude-opus-4-5-20251101', 'Search must find human labels.');
  assert(filterModels(base.models, '20251101')[0] === 'claude-opus-4-5-20251101', 'Search must find full IDs.');
  assert(filterModels(base.models, 'missing').length === 0, 'Search must not invent matches.');
  const ready = render(base);
  assert(ready.includes('role="combobox"') && ready.includes('role="listbox"') && ready.includes('aria-activedescendant='), 'Picker must expose searchable list semantics.');
  assert(ready.includes('aria-selected="true"') && ready.includes('claude-opus-4-5-20251101'), 'Current selection and full IDs must be exposed.');
  const error = render({ ...base, modelsStatus: 'error', modelsError: 'Connection unavailable.', modelSelectionError: 'Check failed.' });
  assert(error.includes('Previously discovered models are still listed.') && error.includes('Check failed.') && error.includes('role="option"'), 'Failures must retain selectable recovery choices.');
  const empty = render({ ...base, models: [] });
  assert(empty.includes('No models returned.') && empty.includes('saved model is unavailable'), 'Empty and unavailable states must offer recovery.');
  assert(render({ ...base, modelsStatus: 'loading' }).includes('Refreshing models'), 'Loading must be announced.');
  const browser = render(base, false);
  assert(browser.includes('desktop app') && !browser.includes('role="option"'), 'Browser preview must not display a model catalog even if passed one.');
  const busy = renderToStaticMarkup(createElement(ModelSelector, { state: { ...base, anyReplyPending: true }, store: nativeStore, onSettings: () => {} }));
  assert(busy.includes('disabled=""') && busy.includes('Opus 5'), 'Model chip must be visible but disabled for active replies.');
  const unchecked = render({ ...base, settings: { ...base.settings!, provider: { ...base.settings!.provider, verified: false } } });
  assert(unchecked.includes('Needs a check') && !unchecked.includes('>Current<'), 'Unchecked selection must not claim success.');
  const hermes: AppSnapshot = { ...base, activeId: 'chat-b', runtimeWorkspace: { workspaceId: 'chat-b', route: 'hermes', modelId: 'accepted-runtime-model', generation: 1, remoteInitialized: true }, runtime: { state: 'ready', message: null, verified: true, generation: 1, capabilities: null, models: [{ id: 'accepted-runtime-model', name: 'Runtime model', available: true }, { id: 'unavailable-runtime-model', name: 'Unavailable', available: false, reason: 'Not approved' }] } };
  const hermesHtml = render(hermes);
  assert(hermesHtml.includes('this workspace only') && hermesHtml.includes('accepted-runtime-model') && !hermesHtml.includes('claude-opus-4-5-20251101') && !hermesHtml.includes('unavailable-runtime-model'), 'Hermes picker must use workspace-accepted model and its own available catalog, never Direct inventory.');
  const applying = renderToStaticMarkup(createElement(ModelSelector, { state: { ...hermes, selectingModel: true }, store: nativeStore, onSettings: () => {} }));
  assert(applying.includes('accepted-runtime-model') && applying.includes('Applying model'), 'Pending model change retains accepted model label.');
  const staleWorkspace = renderToStaticMarkup(createElement(ModelSelector, { state: { ...hermes, activeId: 'chat-c' }, store: nativeStore, onSettings: () => {} }));
  assert(!staleWorkspace.includes('accepted-runtime-model'), 'A previous workspace’s accepted model must not label the new workspace.');
  return ['human labels and full-ID search', 'accessible picker semantics and current model', 'loading, empty, unavailable and error recovery', 'browser catalog denial and global reply lock', 'unchecked selection is not success', 'workspace-scoped Hermes catalog and acknowledged model label'];
}
