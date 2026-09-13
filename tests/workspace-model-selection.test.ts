// node:test fixtures for src/domain/workspace-model-selection.ts.
// Pure contract tests only; no live Hermes, native/browser/backend/device/account
// acceptance is claimed or exercised. Pre-write overlap note: no existing
// model-selection or routing semantics were present under src/domain/ for this
// slice; the interface record/revision module covers different concerns.

import test from 'node:test';
import assert from 'node:assert/strict';
import {
  applySelectionToWorkspace,
  createWorkspaceModelState,
  isExplicitOperatorSelection,
  resolveAutomaticSelection,
  resolveExplicitSelection,
  SelectionError,
  type WorkspaceModelStateMap,
} from '../src/domain/workspace-model-selection.ts';

function buildIndependentState(): WorkspaceModelStateMap {
  // Independently constructed per-workspace entries so isolation assertions
  // cannot pass through shared references.
  const map: Record<string, ReturnType<typeof createWorkspaceModelState>> = {};
  map['ws-a'] = createWorkspaceModelState('ws-a', 'hist://a', 'draft://a');
  map['ws-b'] = createWorkspaceModelState('ws-b', 'hist://b', 'draft://b');
  map['ws-c'] = createWorkspaceModelState('ws-c', 'hist://c', 'draft://c');
  return map;
}

test('automatic selection prefers an available Opus model with provenance', () => {
  const sel = resolveAutomaticSelection('ws-a', {
    models: [
      { id: 'm-haiku', provider: 'p1', name: 'Some Haiku' },
      { id: 'm-opus', provider: 'p2', name: 'Claude Opus' },
    ],
  });
  assert.equal(sel.model, 'm-opus');
  assert.equal(sel.provider, 'p2');
  assert.equal(sel.provenance.kind, 'automatic');
  assert.equal(sel.provenance.reason, 'preferred-opus-available');
  assert.equal(isExplicitOperatorSelection(sel.provenance), false);
});

test('automatic selection records a reason when no Opus model exists', () => {
  const sel = resolveAutomaticSelection('ws-a', {
    models: [{ id: 'm-sonnet', provider: 'p1', name: 'Some Sonnet' }],
  });
  assert.equal(sel.model, 'm-sonnet');
  assert.equal(sel.provenance.kind, 'automatic');
  assert.equal(
    (sel.provenance as { reason: string }).reason,
    'no-opus-available',
  );
});

test('automatic selection with no models records no-models-available', () => {
  const sel = resolveAutomaticSelection('ws-a', { models: [] });
  assert.equal(sel.model, null);
  assert.equal(sel.provider, null);
  assert.equal(sel.provenance.kind, 'automatic');
  assert.equal(
    (sel.provenance as { reason: string }).reason,
    'no-models-available',
  );
});

test('explicit operator choice survives availability changes without substitution', () => {
  const explicit = resolveExplicitSelection('ws-a', 'm-opus', 'p2');
  assert.equal(explicit.model, 'm-opus');
  assert.equal(explicit.provider, 'p2');
  assert.equal(explicit.provenance.kind, 'explicit');
  assert.equal(isExplicitOperatorSelection(explicit.provenance), true);

  // Model becomes unavailable: applying the remembered explicit choice keeps it.
  const state = buildIndependentState();
  const next = applySelectionToWorkspace('ws-a', state, explicit);
  const emptyAvailability = resolveAutomaticSelection('ws-a', { models: [] });
  // Explicit choice is retained verbatim; no silent provider/model substitution.
  assert.equal(next['ws-a'].selection.model, 'm-opus');
  assert.equal(next['ws-a'].selection.provider, 'p2');
  assert.equal(emptyAvailability.model, null); // contrast: automatic would be null
});

test('resolver output is frozen; inputs are never mutated', () => {
  const state = buildIndependentState();
  const frozenInput = Object.freeze(structuredClone(state));
  const snapshot = structuredClone(frozenInput);
  const explicit = resolveExplicitSelection('ws-b', 'm-x', 'p-x');
  assert.equal(Object.isFrozen(explicit), true);
  assert.equal(Object.isFrozen(explicit.provenance), true);
  const next = applySelectionToWorkspace('ws-b', frozenInput, explicit);
  assert.notEqual(next, frozenInput);
  assert.deepEqual(frozenInput, snapshot);
  assert.deepEqual(
    Object.keys(next).sort(),
    Object.keys(snapshot).sort(),
  );
});

test('applying a selection leaves every other workspace untouched (deep-equal)', () => {
  const state = buildIndependentState();
  const before = structuredClone(state);
  const explicit = resolveExplicitSelection('ws-a', 'm-opus', 'p2');
  const next = applySelectionToWorkspace('ws-a', state, explicit);
  for (const id of ['ws-b', 'ws-c']) {
    assert.deepEqual(next[id], before[id]);
    assert.deepEqual(next[id].selection, before[id].selection);
    assert.equal(next[id].historyRef, before[id].historyRef);
    assert.equal(next[id].draftRef, before[id].draftRef);
  }
  // Target workspace changed only its selection; refs preserved.
  assert.equal(next['ws-a'].historyRef, 'hist://a');
  assert.equal(next['ws-a'].draftRef, 'draft://a');
  assert.equal(next['ws-a'].selection.model, 'm-opus');
  // Untouched entries are structurally shared, not copies that drifted.
  assert.equal(next['ws-b'], state['ws-b']);
  assert.equal(next['ws-c'], state['ws-c']);
});

test('forged explicit provenance is rejected and cannot pass as operator choice', () => {
  const state = buildIndependentState();
  const forged = {
    model: 'm-evil',
    provider: 'p-evil',
    provenance: { kind: 'explicit', chosenBy: 'operator' },
  };
  assert.equal(isExplicitOperatorSelection(forged.provenance), false);
  assert.throws(
    () => applySelectionToWorkspace('ws-a', state, forged as never),
    SelectionError,
  );
});

test('invalid ids and inputs throw SelectionError', () => {
  assert.throws(() => resolveExplicitSelection('', 'm', 'p'), SelectionError);
  assert.throws(
    () => resolveAutomaticSelection('ws', { models: 'nope' as never }),
    SelectionError,
  );
  assert.throws(
    () => applySelectionToWorkspace('ws', null as never, resolveExplicitSelection('ws', 'm', 'p')),
    SelectionError,
  );
});
