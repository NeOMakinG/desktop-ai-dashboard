/**
 * Labeled SYNTHETIC fixtures for the custom-code preview bridge envelope contract.
 *
 * Every fixture is invented data. None contains account data, credentials, tokens,
 * cookies, session material, or personal identifiers, and none references any
 * Higgsfield asset, account, model, cost, or generation state.
 *
 * Data-shape validation only: these fixtures provide no evidence of runtime
 * containment, origin enforcement, egress blocking, filesystem or native escape
 * resistance, dependency or compile escape, CPU or allocation termination,
 * watchdog recovery, host recovery timing, or last-valid-revision survival, and
 * imply no independent safety review. Parent C04 remains open.
 */

import {
  PROTOCOL_VERSION,
  type BridgeEnvelope,
  type BridgeRejectionCode,
  type BridgeValidationContext,
  validateBridgeEnvelope,
} from "./customPreviewBridge.js";

export interface LabeledFixture {
  label: string;
  synthetic: true;
  /** Human-readable rejection class the fixture exercises; null for accepted cases. */
  expectedRejection: BridgeRejectionCode | null;
  envelope: unknown;
  context: BridgeValidationContext;
}

export const FIXTURE_CONTEXT: BridgeValidationContext = {
  expectedSessionId: "synthetic-session-alpha",
  expectedOriginId: "synthetic-origin-alpha",
  lastAcceptedSequence: 7,
  maxPayloadBytes: 65536,
};

export function bytesOf(payload: unknown): number {
  const encoded = JSON.stringify(payload);
  return encoded === undefined ? Number.POSITIVE_INFINITY : encoded.length;
}

/**
 * A useful composed custom layout projection: a mixed tabular plus summary-stat
 * projection built entirely from invented values (not a catalog rearrangement).
 */
export const SYNTHETIC_RENDER_PROJECTION_PAYLOAD = {
  layout: "composed:mixed-table-plus-summary",
  table: {
    columns: ["region", "units", "unitCostMicros", "revenueMicros"],
    rows: [
      { region: "north-demo", units: 12, unitCostMicros: 250, revenueMicros: 3000 },
      { region: "south-demo", units: 9, unitCostMicros: 410, revenueMicros: 3690 },
      { region: "east-demo", units: 15, unitCostMicros: 180, revenueMicros: 2700 },
      { region: "west-demo", units: 7, unitCostMicros: 900, revenueMicros: 6300 },
    ],
  },
  summaryStats: {
    totalUnits: 43,
    totalRevenueMicros: 15690,
    meanUnitCostMicros: 435,
    topRegionByRevenue: "west-demo",
  },
  note: "All values invented for synthetic validation fixtures.",
} as const;

function acceptedEnvelope(overrides?: Partial<BridgeEnvelope>): BridgeEnvelope {
  const payload = SYNTHETIC_RENDER_PROJECTION_PAYLOAD;
  return {
    kind: "host:render-projection",
    protocolVersion: PROTOCOL_VERSION,
    sessionId: FIXTURE_CONTEXT.expectedSessionId,
    expectedOriginId: FIXTURE_CONTEXT.expectedOriginId,
    sequence: 8,
    declaredPayloadBytes: bytesOf(payload),
    budget: { cpuMs: 500, memoryBytes: 8388608, messageCount: 64 },
    payload,
    ...overrides,
  };
}

export const SYNTHETIC_ACCEPTED_ENVELOPE: BridgeEnvelope = acceptedEnvelope();

/** Accepted envelopes covering each permitted message kind. */
export const SYNTHETIC_ACCEPTED_BY_KIND: readonly BridgeEnvelope[] = [
  acceptedEnvelope({ kind: "host:render-projection", sequence: 8, payload: SYNTHETIC_RENDER_PROJECTION_PAYLOAD, declaredPayloadBytes: bytesOf(SYNTHETIC_RENDER_PROJECTION_PAYLOAD) }),
  acceptedEnvelope({ kind: "host:revision-identity", sequence: 9, payload: { interfaceId: "synthetic-iface-1", revision: 3 }, declaredPayloadBytes: bytesOf({ interfaceId: "synthetic-iface-1", revision: 3 }) }),
  acceptedEnvelope({ kind: "host:cancel", sequence: 10, payload: { reason: "synthetic-cancel" }, declaredPayloadBytes: bytesOf({ reason: "synthetic-cancel" }) }),
  acceptedEnvelope({ kind: "runtime:ready", sequence: 11, payload: { runtimeName: "synthetic-preview" }, declaredPayloadBytes: bytesOf({ runtimeName: "synthetic-preview" }) }),
  acceptedEnvelope({ kind: "runtime:render-result", sequence: 12, payload: { status: "ok", revision: 4 }, declaredPayloadBytes: bytesOf({ status: "ok", revision: 4 }) }),
  acceptedEnvelope({ kind: "runtime:error", sequence: 13, payload: { message: "synthetic failure" }, declaredPayloadBytes: bytesOf({ message: "synthetic failure" }) }),
  acceptedEnvelope({ kind: "runtime:budget-report", sequence: 14, payload: { cpuMsUsed: 40 }, declaredPayloadBytes: bytesOf({ cpuMsUsed: 40 }) }),
];

const OVERSIZED_ACTUAL_PAYLOAD = { blob: "x".repeat(70000) };

export const SYNTHETIC_FIXTURES: readonly LabeledFixture[] = [
  {
    label: "synthetic accepted composed mixed table + summary projection",
    synthetic: true,
    expectedRejection: null,
    envelope: SYNTHETIC_ACCEPTED_ENVELOPE,
    context: FIXTURE_CONTEXT,
  },
  {
    label: "synthetic malformed shape (primitive input)",
    synthetic: true,
    expectedRejection: "malformed_shape",
    envelope: 42,
    context: FIXTURE_CONTEXT,
  },
  {
    label: "synthetic unknown message kind",
    synthetic: true,
    expectedRejection: "unknown_kind",
    envelope: acceptedEnvelope({ kind: "host:escalate-privileges" as never, sequence: 8 }),
    context: FIXTURE_CONTEXT,
  },
  {
    label: "synthetic version mismatch",
    synthetic: true,
    expectedRejection: "version_mismatch",
    envelope: acceptedEnvelope({ protocolVersion: 999, sequence: 8 }),
    context: FIXTURE_CONTEXT,
  },
  {
    label: "synthetic spoofed session identifier",
    synthetic: true,
    expectedRejection: "session_mismatch",
    envelope: acceptedEnvelope({ sessionId: "synthetic-session-imposter", sequence: 8 }),
    context: FIXTURE_CONTEXT,
  },
  {
    label: "synthetic spoofed origin identifier",
    synthetic: true,
    expectedRejection: "origin_mismatch",
    envelope: acceptedEnvelope({ expectedOriginId: "synthetic-origin-imposter", sequence: 8 }),
    context: FIXTURE_CONTEXT,
  },
  {
    label: "synthetic replayed sequence number",
    synthetic: true,
    expectedRejection: "sequence_replay",
    envelope: acceptedEnvelope({ sequence: 7 }),
    context: FIXTURE_CONTEXT,
  },
  {
    label: "synthetic regressed sequence number",
    synthetic: true,
    expectedRejection: "sequence_regression",
    envelope: acceptedEnvelope({ sequence: 3 }),
    context: FIXTURE_CONTEXT,
  },
  {
    label: "synthetic declared payload length over cap",
    synthetic: true,
    expectedRejection: "declared_size_over_cap",
    envelope: acceptedEnvelope({ sequence: 8, declaredPayloadBytes: 65537, payload: "y".repeat(65537), declaredPayloadBytesOverride: undefined } as never),
    context: FIXTURE_CONTEXT,
  },
  {
    label: "synthetic actual payload size over cap (accurate declaration)",
    synthetic: true,
    expectedRejection: "actual_size_over_cap",
    envelope: acceptedEnvelope({ sequence: 8, payload: OVERSIZED_ACTUAL_PAYLOAD, declaredPayloadBytes: bytesOf(OVERSIZED_ACTUAL_PAYLOAD) }),
    context: FIXTURE_CONTEXT,
  },
  {
    label: "synthetic declared length disagrees with actual size",
    synthetic: true,
    expectedRejection: "declared_size_mismatch",
    envelope: acceptedEnvelope({ sequence: 8, declaredPayloadBytes: 5 }),
    context: FIXTURE_CONTEXT,
  },
  {
    label: "synthetic missing budget declaration",
    synthetic: true,
    expectedRejection: "missing_budget",
    envelope: (() => {
      const e = acceptedEnvelope({ sequence: 8 }) as unknown as Record<string, unknown>;
      delete e.budget;
      return e;
    })(),
    context: FIXTURE_CONTEXT,
  },
  {
    label: "synthetic non-finite budget field",
    synthetic: true,
    expectedRejection: "non_finite_budget",
    envelope: acceptedEnvelope({ sequence: 8, budget: { cpuMs: Number.POSITIVE_INFINITY, memoryBytes: 1024, messageCount: 8 } }),
    context: FIXTURE_CONTEXT,
  },
  {
    label: "synthetic non-integer budget field",
    synthetic: true,
    expectedRejection: "non_positive_or_non_integer_budget",
    envelope: acceptedEnvelope({ sequence: 8, budget: { cpuMs: 100.5, memoryBytes: 1024, messageCount: 8 } }),
    context: FIXTURE_CONTEXT,
  },
  {
    label: "synthetic zero (non-positive) budget field",
    synthetic: true,
    expectedRejection: "non_positive_or_non_integer_budget",
    envelope: acceptedEnvelope({ sequence: 8, budget: { cpuMs: 100, memoryBytes: 0, messageCount: 8 } }),
    context: FIXTURE_CONTEXT,
  },
];

/**
 * Convenience: run the pure validator over every fixture. Never throws.
 */
export function validateAllFixtures(): { label: string; result: ReturnType<typeof validateBridgeEnvelope> }[] {
  return SYNTHETIC_FIXTURES.map((f) => ({
    label: f.label,
    result: validateBridgeEnvelope(f.envelope, f.context),
  }));
}
