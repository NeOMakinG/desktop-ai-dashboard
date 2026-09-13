// Pure domain fixtures for per-workspace model selection (HI02 spike slice).
// Source-level contract only: no renderer, native, OS, backend, or runtime-process
// code; no live Hermes, native/browser/backend/device/account acceptance claimed.
// Erasable TypeScript only (interfaces, type aliases) so Node 22 type-stripping
// executes this file directly with no build step.

export interface ModelDescriptor {
  readonly id: string;
  readonly provider: string;
  readonly name: string;
}

export interface AvailabilityInput {
  readonly models: readonly ModelDescriptor[];
}

export type AutomaticReason =
  | 'preferred-opus-available'
  | 'no-opus-available'
  | 'no-models-available';

export interface ExplicitProvenance {
  readonly kind: 'explicit';
  readonly chosenBy: 'operator';
}

export interface AutomaticProvenance {
  readonly kind: 'automatic';
  readonly reason: AutomaticReason;
}

export type SelectionProvenance = ExplicitProvenance | AutomaticProvenance;

export interface ModelSelection {
  readonly model: string | null;
  readonly provider: string | null;
  readonly provenance: SelectionProvenance;
}

export interface WorkspaceModelState {
  readonly selection: ModelSelection;
  readonly historyRef: string;
  readonly draftRef: string;
}

export type WorkspaceModelStateMap = Readonly<Record<string, WorkspaceModelState>>;

export const EXPLICIT_BRAND = Symbol.for('forma.domain.explicitSelection');

export class SelectionError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'SelectionError';
  }
}

function requireNonEmptyString(value: unknown, label: string): string {
  if (typeof value !== 'string' || value.length === 0) {
    throw new SelectionError(`invalid ${label}: expected a non-empty string`);
  }
  return value;
}

function isOpus(model: ModelDescriptor): boolean {
  return model.name.toLowerCase().includes('opus');
}

// Provenance objects are frozen and, for explicit choices, carry a non-enumerable
// brand symbol that only this module's resolver path attaches. Callers cannot
// forge an automatic selection as an explicit operator choice by copying fields.
function makeExplicitProvenance(): ExplicitProvenance {
  const p = { kind: 'explicit', chosenBy: 'operator' } as ExplicitProvenance;
  Object.defineProperty(p, EXPLICIT_BRAND, {
    value: true,
    enumerable: false,
    writable: false,
    configurable: false,
  });
  return Object.freeze(p);
}

function makeAutomaticProvenance(reason: AutomaticReason): AutomaticProvenance {
  return Object.freeze({ kind: 'automatic', reason });
}

export function isExplicitOperatorSelection(
  provenance: SelectionProvenance,
): boolean {
  return (
    provenance.kind === 'explicit' &&
    (provenance as unknown as Record<symbol, unknown>)[EXPLICIT_BRAND] === true
  );
}

function freezeSelection(selection: ModelSelection): ModelSelection {
  return Object.freeze({
    model: selection.model,
    provider: selection.provider,
    provenance: selection.provenance,
  });
}

// Explicit operator choice. Survives availability changes: the choice is
// recorded verbatim (model + provider) even when the model is not currently
// available; nothing is silently substituted.
export function resolveExplicitSelection(
  workspaceId: string,
  model: string,
  provider: string,
): ModelSelection {
  requireNonEmptyString(workspaceId, 'workspaceId');
  requireNonEmptyString(model, 'model');
  requireNonEmptyString(provider, 'provider');
  return freezeSelection({
    model,
    provider,
    provenance: makeExplicitProvenance(),
  });
}

// Automatic selection: prefers an available Opus model; otherwise records a
// reason ('no-opus-available' picks the first available model; 'no-models-available'
// leaves model/provider null while still carrying inspectable provenance).
export function resolveAutomaticSelection(
  workspaceId: string,
  availability: AvailabilityInput,
): ModelSelection {
  requireNonEmptyString(workspaceId, 'workspaceId');
  if (availability === null || typeof availability !== 'object') {
    throw new SelectionError('invalid availability input');
  }
  const models = availability.models;
  if (!Array.isArray(models)) {
    throw new SelectionError('invalid availability input: models must be an array');
  }
  for (const m of models) {
    if (
      m === null ||
      typeof m !== 'object' ||
      typeof m.id !== 'string' ||
      typeof m.provider !== 'string' ||
      typeof m.name !== 'string'
    ) {
      throw new SelectionError('invalid model descriptor in availability input');
    }
  }
  if (models.length === 0) {
    return freezeSelection({
      model: null,
      provider: null,
      provenance: makeAutomaticProvenance('no-models-available'),
    });
  }
  const opus = models.find(isOpus);
  if (opus !== undefined) {
    return freezeSelection({
      model: opus.id,
      provider: opus.provider,
      provenance: makeAutomaticProvenance('preferred-opus-available'),
    });
  }
  const first = models[0];
  return freezeSelection({
    model: first.id,
    provider: first.provider,
    provenance: makeAutomaticProvenance('no-opus-available'),
  });
}

// Pure application: returns a new state map; never mutates the input. Only the
// target workspace's entry is replaced; every other workspace's selection,
// history reference, and draft reference keeps its prior value (structural
// sharing means untouched entries are the same frozen objects).
export function applySelectionToWorkspace(
  state: WorkspaceModelStateMap,
  workspaceId: string,
  selection: ModelSelection,
): WorkspaceModelStateMap {
  requireNonEmptyString(workspaceId, 'workspaceId');
  if (state === null || typeof state !== 'object') {
    throw new SelectionError('invalid state map');
  }
  if (
    selection === null ||
    typeof selection !== 'object' ||
    typeof selection.provenance !== 'object' ||
    selection.provenance === null ||
    typeof selection.provenance.kind !== 'string'
  ) {
    throw new SelectionError('invalid selection');
  }
  if (selection.provenance.kind === 'explicit') {
    // An explicit provenance not minted by the resolver cannot carry the brand;
    // reject forged explicit selections instead of accepting misleading success.
    if (!isExplicitOperatorSelection(selection.provenance)) {
      throw new SelectionError(
        'forged explicit provenance: explicit selections must come from resolveExplicitSelection',
      );
    }
  }
  if (selection.provenance.kind === 'automatic') {
    const reason = (selection.provenance as AutomaticProvenance).reason;
    if (
      reason !== 'preferred-opus-available' &&
      reason !== 'no-opus-available' &&
      reason !== 'no-models-available'
    ) {
      throw new SelectionError('invalid automatic provenance reason');
    }
  }
  const prior = state[workspaceId];
  const next: WorkspaceModelState = Object.freeze({
    selection: freezeSelection(selection),
    historyRef: prior === undefined ? '' : prior.historyRef,
    draftRef: prior === undefined ? '' : prior.draftRef,
  });
  const nextMap: Record<string, WorkspaceModelState> = {};
  for (const key of Object.keys(state)) {
    nextMap[key] = state[key];
  }
  nextMap[workspaceId] = next;
  return nextMap;
}

// Convenience helper that builds an initial frozen workspace entry.
export function createWorkspaceModelState(
  workspaceId: string,
  historyRef: string,
  draftRef: string,
): WorkspaceModelState {
  requireNonEmptyString(workspaceId, 'workspaceId');
  requireNonEmptyString(historyRef, 'historyRef');
  requireNonEmptyString(draftRef, 'draftRef');
  return Object.freeze({
    selection: Object.freeze({
      model: null,
      provider: null,
      provenance: makeAutomaticProvenance('no-models-available'),
    }),
    historyRef,
    draftRef,
  });
}
