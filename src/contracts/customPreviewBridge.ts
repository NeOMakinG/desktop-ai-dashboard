/**
 * Pure envelope-validation contract for the host <-> contained custom-code preview
 * runtime bridge. Data-shape validation only.
 *
 * This module provides NO evidence of runtime containment, origin enforcement,
 * egress blocking, filesystem or native escape resistance, dependency or compile
 * escape, CPU or allocation termination, watchdog recovery, host recovery timing,
 * or last-valid-revision survival, and implies no independent safety review.
 * Parent C04 remains open; custom preview remains disabled.
 *
 * Dependency-free and side-effect-free: no network, filesystem, clock, randomness,
 * DOM, native, or process access; no module-level mutable state. Replay and
 * sequence checks use only caller-supplied values.
 */

export const PROTOCOL_VERSION = 1 as const;

export const DEFAULT_MAX_PAYLOAD_BYTES = 65536 as const;

export type HostToRuntimeKind =
  | "host:render-projection"
  | "host:revision-identity"
  | "host:cancel";

export type RuntimeToHostKind =
  | "runtime:ready"
  | "runtime:render-result"
  | "runtime:error"
  | "runtime:budget-report";

export type BridgeMessageKind = HostToRuntimeKind | RuntimeToHostKind;

export const BRIDGE_MESSAGE_KINDS: readonly BridgeMessageKind[] = [
  "host:render-projection",
  "host:revision-identity",
  "host:cancel",
  "runtime:ready",
  "runtime:render-result",
  "runtime:error",
  "runtime:budget-report",
];

export interface BridgeBudgetDeclaration {
  /** CPU milliseconds. Must be a finite, positive integer. */
  cpuMs: number;
  /** Memory bytes. Must be a finite, positive integer. */
  memoryBytes: number;
  /** Message count. Must be a finite, positive integer. */
  messageCount: number;
}

export interface BridgeEnvelope {
  kind: BridgeMessageKind;
  protocolVersion: number;
  /** Opaque session identifier; compared only against the caller's expectation. */
  sessionId: string;
  /** Opaque expected-origin identifier; compared only against the caller's expectation. */
  expectedOriginId: string;
  /** Monotonically increasing integer sequence number. */
  sequence: number;
  /** Declared payload byte length. */
  declaredPayloadBytes: number;
  budget: BridgeBudgetDeclaration;
  /** Arbitrary JSON-shaped payload; its encoded byte length is measured. */
  payload: unknown;
}

export type BridgeRejectionCode =
  | "unknown_kind"
  | "version_mismatch"
  | "session_mismatch"
  | "origin_mismatch"
  | "sequence_replay"
  | "sequence_regression"
  | "declared_size_over_cap"
  | "actual_size_over_cap"
  | "declared_size_mismatch"
  | "malformed_shape"
  | "missing_budget"
  | "non_finite_budget"
  | "non_positive_or_non_integer_budget";

export type BridgeValidationResult =
  | { ok: true; envelope: BridgeEnvelope }
  | { ok: false; code: BridgeRejectionCode };

export interface BridgeValidationContext {
  expectedSessionId: string;
  expectedOriginId: string;
  /** Last accepted sequence number for this session; replay/regression compare against it. */
  lastAcceptedSequence?: number;
  maxPayloadBytes?: number;
}

function payloadByteLength(payload: unknown): number {
  const encoded = JSON.stringify(payload);
  if (encoded === undefined) {
    return Number.POSITIVE_INFINITY;
  }
  return encoded.length;
}

function isPlainFiniteIntegerAtLeast(n: unknown, min: number): boolean {
  return (
    typeof n === "number" &&
    Number.isFinite(n) &&
    Number.isInteger(n) &&
    n >= min
  );
}

/**
 * Pure validator. Returns a discriminated result and never throws for any input,
 * including null, undefined, primitives, arrays, and prototype-polluted objects.
 */
export function validateBridgeEnvelope(
  input: unknown,
  context: BridgeValidationContext,
): BridgeValidationResult {
  try {
    if (
      input === null ||
      typeof input !== "object" ||
      Array.isArray(input)
    ) {
      return { ok: false, code: "malformed_shape" };
    }

    const candidate = input as Record<string, unknown>;

    if (typeof candidate.kind !== "string") {
      return { ok: false, code: "malformed_shape" };
    }
    if (!(BRIDGE_MESSAGE_KINDS as readonly string[]).includes(candidate.kind)) {
      return { ok: false, code: "unknown_kind" };
    }

    if (
      candidate.protocolVersion !== PROTOCOL_VERSION ||
      typeof candidate.sessionId !== "string" ||
      typeof candidate.expectedOriginId !== "string" ||
      !isPlainFiniteIntegerAtLeast(candidate.sequence, 0) ||
      !isPlainFiniteIntegerAtLeast(candidate.declaredPayloadBytes, 0) ||
      !Object.prototype.hasOwnProperty.call(candidate, "payload")
    ) {
      return { ok: false, code: "malformed_shape" };
    }

    if (candidate.sessionId !== context.expectedSessionId) {
      return { ok: false, code: "session_mismatch" };
    }
    if (candidate.expectedOriginId !== context.expectedOriginId) {
      return { ok: false, code: "origin_mismatch" };
    }

    const last = context.lastAcceptedSequence;
    if (typeof last === "number" && Number.isFinite(last)) {
      if (candidate.sequence === last) {
        return { ok: false, code: "sequence_replay" };
      }
      if ((candidate.sequence as number) < last) {
        return { ok: false, code: "sequence_regression" };
      }
    }

    const budget = candidate.budget;
    if (
      budget === null ||
      typeof budget !== "object" ||
      Array.isArray(budget)
    ) {
      return { ok: false, code: "missing_budget" };
    }
    const b = budget as Record<string, unknown>;
    const fields = [b.cpuMs, b.memoryBytes, b.messageCount];
    if (
      fields.some(
        (f) => typeof f !== "number" || !Object.prototype.hasOwnProperty.call(b, "cpuMs") === false,
      )
    ) {
      // placeholder branch replaced below
    }
    if (
      !Object.prototype.hasOwnProperty.call(b, "cpuMs") ||
      !Object.prototype.hasOwnProperty.call(b, "memoryBytes") ||
      !Object.prototype.hasOwnProperty.call(b, "messageCount")
    ) {
      return { ok: false, code: "missing_budget" };
    }
    if (fields.some((f) => typeof f !== "number" || !Number.isFinite(f))) {
      return { ok: false, code: "non_finite_budget" };
    }
    if (fields.some((f) => !isPlainFiniteIntegerAtLeast(f, 1))) {
      return { ok: false, code: "non_positive_or_non_integer_budget" };
    }

    const cap =
      typeof context.maxPayloadBytes === "number" && Number.isFinite(context.maxPayloadBytes)
        ? context.maxPayloadBytes
        : DEFAULT_MAX_PAYLOAD_BYTES;

    const actualBytes = payloadByteLength(candidate.payload);
    if (actualBytes > cap) {
      return { ok: false, code: "actual_size_over_cap" };
    }
    if ((candidate.declaredPayloadBytes as number) > cap) {
      return { ok: false, code: "declared_size_over_cap" };
    }
    if ((candidate.declaredPayloadBytes as number) !== actualBytes) {
      return { ok: false, code: "declared_size_mismatch" };
    }

    const envelope: BridgeEnvelope = {
      kind: candidate.kind as BridgeMessageKind,
      protocolVersion: candidate.protocolVersion as number,
      sessionId: candidate.sessionId,
      expectedOriginId: candidate.expectedOriginId,
      sequence: candidate.sequence as number,
      declaredPayloadBytes: candidate.declaredPayloadBytes as number,
      budget: {
        cpuMs: b.cpuMs as number,
        memoryBytes: b.memoryBytes as number,
        messageCount: b.messageCount as number,
      },
      payload: candidate.payload,
    };
    return { ok: true, envelope };
  } catch {
    return { ok: false, code: "malformed_shape" };
  }
}
