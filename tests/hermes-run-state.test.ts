// node:test fixtures for src/domain/hermes-run-state.ts.
// Pure contract tests only; no live Hermes, native/browser/backend/device/account
// acceptance is claimed or exercised.

import test from 'node:test';
import assert from 'node:assert/strict';
import {
  assertEventBelongsToRun,
  assertRunBelongsToSession,
  createRunEvent,
  createRunState,
  IllegalTransitionError,
  LIFECYCLE_TRANSITIONS,
  RECONNECT_STATES,
  RunIdentityError,
  transitionRunState,
  type RunLifecycle,
} from '../src/domain/hermes-run-state.ts';

test('module smoke: exported symbols import and the transition table parses', () => {
  assert.equal(typeof transitionRunState, 'function');
  assert.equal(typeof createRunState, 'function');
  assert.equal(Object.keys(LIFECYCLE_TRANSITIONS).length, 8);
  assert.deepEqual([...RECONNECT_STATES], [
    'starting',
    'ready',
    'recovering',
    'unavailable',
  ]);
});

test('a run references exactly one session; malformed ids throw', () => {
  const run = createRunState('run-1', 'sess-1', 'starting');
  assert.equal(run.identity.runId, 'run-1');
  assert.equal(run.identity.sessionId, 'sess-1');
  assert.ok(run.identity === run.identity); // single session reference
  assert.equal(Object.keys(run.identity).length, 2);
  assert.doesNotThrow(() => assertRunBelongsToSession(run, 'sess-1'));
  assert.throws(
    () => assertRunBelongsToSession(run, 'sess-other'),
    RunIdentityError,
  );
  assert.throws(() => createRunState('', 'sess-1', 'starting'), RunIdentityError);
  assert.throws(() => createRunState('run-1', '', 'starting'), RunIdentityError);
  assert.throws(
    () => createRunState('run-1', 'sess-1', 'bogus' as RunLifecycle),
    RunIdentityError,
  );
});

test('an event references its owning run; malformed ids throw', () => {
  const event = createRunEvent('ev-1', 'run-1', 'text');
  assert.doesNotThrow(() => assertEventBelongsToRun(event, 'run-1'));
  assert.throws(
    () => assertEventBelongsToRun(event, 'run-2'),
    RunIdentityError,
  );
  assert.throws(() => createRunEvent('', 'run-1', 'text'), RunIdentityError);
  assert.throws(() => createRunEvent('ev-1', '', 'text'), RunIdentityError);
});

test('legal transitions follow the explicit table', () => {
  let run = createRunState('run-1', 'sess-1', 'starting');
  run = transitionRunState(run, 'ready');
  run = transitionRunState(run, 'running');
  run = transitionRunState(run, 'recovering');
  run = transitionRunState(run, 'ready');
  run = transitionRunState(run, 'running');
  run = transitionRunState(run, 'completed');
  assert.equal(run.lifecycle, 'completed');
  assert.equal(run.identity.runId, 'run-1');
  assert.equal(run.identity.sessionId, 'sess-1');
});

test('cancelled and interrupted runs can never transition to completed', () => {
  for (const terminal of ['cancelled', 'interrupted'] as const) {
    const run = createRunState('run-1', 'sess-1', 'running');
    const stopped = transitionRunState(run, terminal);
    assert.throws(
      () => transitionRunState(stopped, 'completed'),
      (err: unknown) => err instanceof IllegalTransitionError,
    );
    // Terminal states have no outgoing transitions at all.
    assert.deepEqual([...LIFECYCLE_TRANSITIONS[terminal]], []);
    for (const to of Object.keys(LIFECYCLE_TRANSITIONS) as RunLifecycle[]) {
      assert.throws(() => transitionRunState(stopped, to), IllegalTransitionError);
    }
  }
});

test('illegal transitions throw typed errors, including completed->running', () => {
  const completed = createRunState('run-1', 'sess-1', 'completed');
  assert.throws(
    () => transitionRunState(completed, 'running'),
    IllegalTransitionError,
  );
  const ready = createRunState('run-1', 'sess-1', 'ready');
  assert.throws(
    () => transitionRunState(ready, 'completed'),
    (err: unknown) =>
      err instanceof IllegalTransitionError &&
      err.from === 'ready' &&
      err.to === 'completed',
  );
  assert.throws(
    () => transitionRunState(null as never, 'ready'),
    RunIdentityError,
  );
  assert.throws(
    () => transitionRunState(ready, 'nonsense' as RunLifecycle),
    RunIdentityError,
  );
});

test('reconnect states expose only starting/ready/recovering/unavailable transitions', () => {
  assert.deepEqual([...LIFECYCLE_TRANSITIONS['starting']], [
    'ready',
    'unavailable',
  ]);
  assert.deepEqual([...LIFECYCLE_TRANSITIONS['unavailable']], [
    'starting',
    'ready',
  ]);
  const recovering = createRunState('run-1', 'sess-1', 'recovering');
  const back = transitionRunState(recovering, 'running');
  assert.equal(back.lifecycle, 'running');
});

test('transitionRunState returns a new frozen object and never mutates input', () => {
  const run = createRunState('run-1', 'sess-1', 'starting');
  const next = transitionRunState(run, 'ready');
  assert.notEqual(next, run);
  assert.equal(run.lifecycle, 'starting');
  assert.equal(next.lifecycle, 'ready');
  assert.equal(Object.isFrozen(run), true);
  assert.equal(Object.isFrozen(next), true);
});
