/**
 * Pure data-plane pinning contract for workspace history (F08).
 *
 * No I/O, no UI, no store coupling. Pin state is a plain per-workspace
 * record intended to travel alongside workspace state through the
 * store's existing persistence path (wiring is a later job).
 *
 * All functions are pure: inputs are never mutated.
 */

export type WorkspaceId = string;

/** A workspace as the history list sees it. */
export interface WorkspaceSummary {
  readonly id: WorkspaceId;
  /** Epoch ms of the most recent activity; higher is more recent. */
  readonly lastActivityAt: number;
}

/** Pin state: per-workspace, presence means pinned. */
export interface WorkspacePinState {
  readonly pins: Readonly<Record<WorkspaceId, true>>;
}

export const emptyPinState = (): WorkspacePinState => ({ pins: {} });

/** Typed errors; no exceptions cross this API. */
export type WorkspacePinError =
  | { readonly kind: "invalid_workspace_id"; readonly workspaceId: string }
  | { readonly kind: "workspace_not_found"; readonly workspaceId: WorkspaceId };

export type Result<T> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly error: WorkspacePinError };

const ok = <T>(value: T): Result<T> => ({ ok: true, value });
const err = (error: WorkspacePinError): Result<never> => ({
  ok: false,
  error,
});

const isValidId = (id: string): id is WorkspaceId =>
  typeof id === "string" && id.length > 0;

/**
 * Whether a workspace is currently pinned. Unknown or unpinned ids are
 * simply unpinned; this never errors.
 */
export const isPinned = (
  state: WorkspacePinState,
  workspaceId: WorkspaceId,
): boolean => state.pins[workspaceId] === true;

/**
 * Toggle the pin for a workspace. Toggling an already-pinned workspace off
 * removes the pin; toggling an already-unpinned workspace on adds it.
 * Re-pinning a pinned workspace is idempotent and returns the current
 * state unchanged.
 *
 * `knownWorkspaces` is the set of ids that currently exist; toggling an
 * unknown id returns a typed `workspace_not_found` error rather than
 * creating an orphan pin.
 */
export const togglePin = (
  state: WorkspacePinState,
  workspaceId: WorkspaceId,
  knownWorkspaces: readonly WorkspaceId[] = [],
): Result<{ readonly state: WorkspacePinState; readonly pinned: boolean }> => {
  if (!isValidId(workspaceId)) {
    return err({ kind: "invalid_workspace_id", workspaceId });
  }
  if (
    knownWorkspaces.length > 0 &&
    !knownWorkspaces.includes(workspaceId)
  ) {
    return err({ kind: "workspace_not_found", workspaceId });
  }
  const currently = isPinned(state, workspaceId);
  if (currently) {
    const pins = { ...state.pins };
    delete pins[workspaceId];
    return ok({ state: { pins }, pinned: false });
  }
  return ok({
    state: { pins: { ...state.pins, [workspaceId]: true as const } },
    pinned: true,
  });
};

/**
 * Pin a workspace without toggling: idempotent. Returns the same state
 * reference when already pinned.
 */
export const pin = (
  state: WorkspacePinState,
  workspaceId: WorkspaceId,
  knownWorkspaces: readonly WorkspaceId[] = [],
): Result<{ readonly state: WorkspacePinState; readonly pinned: boolean }> => {
  if (!isValidId(workspaceId)) {
    return err({ kind: "invalid_workspace_id", workspaceId });
  }
  if (knownWorkspaces.length > 0 && !knownWorkspaces.includes(workspaceId)) {
    return err({ kind: "workspace_not_found", workspaceId });
  }
  if (isPinned(state, workspaceId)) {
    return ok({ state, pinned: true });
  }
  return ok({
    state: { pins: { ...state.pins, [workspaceId]: true as const } },
    pinned: true,
  });
};

/**
 * Deletion cascade: removing a workspace must automatically drop its pin
 * so no orphan entry survives and ordering cannot resurrect it. Idempotent
 * for workspaces that were never pinned.
 */
export const dropPin = (
  state: WorkspacePinState,
  workspaceId: WorkspaceId,
): WorkspacePinState => {
  if (!isPinned(state, workspaceId)) {
    return state;
  }
  const pins = { ...state.pins };
  delete pins[workspaceId];
  return { pins };
};

/**
 * History-list comparator implementing the ordering contract:
 *
 * 1. Pinned workspaces always sort above unpinned ones.
 * 2. Among pinned, order by most recent activity (descending).
 * 3. Among unpinned, order by recency (descending), exactly as today.
 * 4. Deterministic tie-break on workspace id (ascending) so equal
 *    activity timestamps never produce unstable order.
 *
 * For sort() use: returns negative when `a` should come first.
 */
export const pinnedOrdering = (
  pinnedState: WorkspacePinState,
): ((a: WorkspaceSummary, b: WorkspaceSummary) => number) => {
  return (a, b) => {
    const pa = isPinned(pinnedState, a.id) ? 1 : 0;
    const pb = isPinned(pinnedState, b.id) ? 1 : 0;
    if (pa !== pb) return pb - pa; // pinned (1) before unpinned (0)
    if (a.lastActivityAt !== b.lastActivityAt) {
      return b.lastActivityAt - a.lastActivityAt; // most recent first
    }
    if (a.id < b.id) return -1;
    if (a.id > b.id) return 1;
    return 0;
  };
};

/**
 * Convenience: return a NEW array of workspaces ordered by the pinning
 * contract. Never mutates the input array.
 */
export const orderWorkspaceHistory = (
  workspaces: readonly WorkspaceSummary[],
  pinnedState: WorkspacePinState,
): WorkspaceSummary[] =>
  [...workspaces].sort(pinnedOrdering(pinnedState));
