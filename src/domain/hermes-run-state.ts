// Pure domain fixtures for correlated session/run/event identity and the
// run-state machine (HI02 spike slice). Source-level contract only: no renderer,
// native, OS, backend, or runtime-process code; no live Hermes,
// native/browser/backend/device/account acceptance claimed. Erasable TypeScript
// only so Node 22 type-stripping executes this file directly.

export type RunLifecycle =
  | 'running'
  | 'cancelled'
  | 'interrupted'
  | 'completed'
  | 'starting'
  | 'ready'
  | 'recovering'
  | 'unavailable';

export interface RunIdentity {
  readonly runId: string;
  readonly sessionId: string; // a run references exactly one session
}

export interface RunState {
  readonly identity: RunIdentity;
  readonly lifecycle: RunLifecycle;
}

export interface RunEvent {
  readonly eventId: string;
  readonly runId: string; // an event references its owning run
  readonly kind: string;
}

// Explicit allowed-transition table. cancelled, interrupted, and completed are
// terminal with no outgoing transitions, so a cancelled or interrupted run can
// never transition to completed.
export const LIFECYCLE_TRANSITIONS: Readonly<
  Record<RunLifecycle, readonly RunLifecycle[]>
> = Object.freeze({
  starting: Object.freeze(['ready', 'unavailable']),
  ready: Object.freeze(['running', 'recovering', 'unavailable']),
  running: Object.freeze(['completed', 'cancelled', 'interrupted', 'recovering']),
  recovering: Object.freeze(['running', 'ready', 'unavailable']),
  unavailable: Object.freeze(['starting', 'ready']),
  cancelled: Object.freeze([]),
  interrupted: Object.freeze([]),
  completed: Object.freeze([]),
});

export const RECONNECT_STATES: readonly RunLifecycle[] = Object.freeze([
  'starting',
  'ready',
  'recovering',
  'unavailable',
]);

export class RunIdentityError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'RunIdentityError';
  }
}

export class IllegalTransitionError extends Error {
  readonly from: RunLifecycle;
  readonly to: RunLifecycle;
  constructor(from: RunLifecycle, to: RunLifecycle) {
    super(`illegal run lifecycle transition: ${from} -> ${to}`);
    this.name = 'IllegalTransitionError';
    this.from = from;
    this.to = to;
  }
}

function requireId(value: unknown, label: string): string {
  if (typeof value !== 'string' || value.length === 0) {
    throw new RunIdentityError(`invalid ${label}: expected a non-empty string`);
  }
  return value;
}

function requireLifecycle(value: unknown): RunLifecycle {
  const known: readonly RunLifecycle[] = [
    'running',
    'cancelled',
    'interrupted',
    'completed',
    'starting',
    'ready',
    'recovering',
    'unavailable',
  ];
  if (typeof value !== 'string' || !known.includes(value as RunLifecycle)) {
    throw new RunIdentityError(`unknown lifecycle state: ${String(value)}`);
  }
  return value as RunLifecycle;
}

// A run references exactly one session; malformed ids throw instead of success.
export function createRunState(
  runId: string,
  sessionId: string,
  lifecycle: RunLifecycle,
): RunState {
  requireId(runId, 'runId');
  requireId(sessionId, 'sessionId');
  requireLifecycle(lifecycle);
  return Object.freeze({
    identity: Object.freeze({ runId, sessionId }),
    lifecycle,
  });
}

// An event references its owning run; malformed ids throw instead of success.
export function createRunEvent(
  eventId: string,
  runId: string,
  kind: string,
): RunEvent {
  requireId(eventId, 'eventId');
  requireId(runId, 'runId');
  requireId(kind, 'kind');
  return Object.freeze({ eventId, runId, kind });
}

// Validates that a run belongs to the given session (correlated identity).
export function assertRunBelongsToSession(
  run: RunState,
  sessionId: string,
): void {
  requireId(sessionId, 'sessionId');
  if (
    run === null ||
    typeof run !== 'object' ||
    run.identity === null ||
    typeof run.identity !== 'object'
  ) {
    throw new RunIdentityError('malformed run state');
  }
  if (run.identity.sessionId !== sessionId) {
    throw new RunIdentityError(
      `run ${run.identity.runId} does not belong to session ${sessionId}`,
    );
  }
}

// Validates that an event references its owning run.
export function assertEventBelongsToRun(event: RunEvent, runId: string): void {
  requireId(runId, 'runId');
  if (event === null || typeof event !== 'object') {
    throw new RunIdentityError('malformed run event');
  }
  if (event.runId !== runId) {
    throw new RunIdentityError(
      `event ${event.eventId} does not belong to run ${runId}`,
    );
  }
}

// Pure transition: returns a new RunState; illegal transitions throw
// IllegalTransitionError rather than returning misleading success.
export function transitionRunState(
  run: RunState,
  to: RunLifecycle,
): RunState {
  if (run === null || typeof run !== 'object') {
    throw new RunIdentityError('malformed run state');
  }
  requireLifecycle(run.lifecycle);
  requireLifecycle(to);
  const allowed = LIFECYCLE_TRANSITIONS[run.lifecycle];
  if (!allowed.includes(to)) {
    throw new IllegalTransitionError(run.lifecycle, to);
  }
  return Object.freeze({
    identity: run.identity,
    lifecycle: to,
  });
}
