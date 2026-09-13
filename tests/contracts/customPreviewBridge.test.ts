/**
 * node:test suite for the custom-code preview bridge envelope contract.
 *
 * Data-shape validation only: passing these tests provides no evidence of
 * runtime containment, origin enforcement, egress blocking, filesystem or
 * native escape resistance, dependency or compile escape, CPU or allocation
 * termination, watchdog recovery, host recovery timing, or last-valid-revision
 * survival, and implies no independent safety review. Parent C04 remains open.
 */

import test from "node:test";
import assert from "node:assert/strict";

import {
  PROTOCOL_VERSION,
  DEFAULT_MAX_PAYLOAD_BYTES,
  validateBridgeEnvelope,
  type BridgeEnvelope,
  type BridgeRejectionCode,
  type BridgeValidationContext,
} from "../../src/contracts/customPreviewBridge.js";

import {
  SYNTHETIC_ACCEPTED_BY_KIND,
  SYNTHETIC_ACCEPTED_ENVELOPE,
  SYNTHETIC_FIXTURES,
  bytesOf,
  FIXTURE_CONTEXT,
  SYNTHETIC_RENDER_PROJECTION_PAYLOAD,
} from "../../src/contracts/customPreviewBridge.fixtures.js";

function acceptedCtx(): BridgeValidationContext {
  return { ...FIXTURE_CONTEXT };
}

function baseEnvelope(): BridgeEnvelope {
  return JSON.parse(JSON.stringify(SYNTHETIC_ACCEPTED_ENVELOPE)) as BridgeEnvelope;
}

const requiredCodes: readonly BridgeRejectionCode[] = [
  "unknown_kind",
  "version_mismatch",
  "session_mismatch",
  "origin_mismatch",
  "sequence_replay",
  "sequence_regression",
  "declared_size_over_cap",
  "actual_size_over_cap",
  "declared_size_mismatch",
  "malformed_shape",
  "missing_budget",
  "non_finite_budget",
  "non_positive_or_non_integer_budget",
];

test("accepted composed mixed table + summary projection envelope is narrowed", () => {
  const result = validateBridgeEnvelope(SYNTHETIC_ACCEPTED_ENVELOPE, acceptedCtx());
  assert.equal(result.ok, true);
  assert.ok(result.ok);
  assert.equal(result.envelope.kind, "host:render-projection");
  assert.equal(result.envelope.protocolVersion, PROTOCOL_VERSION);
  assert.equal(result.envelope.sequence, 8);
  assert.equal(result.envelope.declaredPayloadBytes, bytesOf(SYNTHETIC_RENDER_PROJECTION_PAYLOAD));
  assert.equal(result.envelope.budget.cpuMs, 500);
  assert.equal(result.envelope.budget.memoryBytes, 8388608);
  assert.equal(result.envelope.budget.messageCount, 64);
  assert.deepEqual(result.envelope.payload, SYNTHETIC_RENDER_PROJECTION_PAYLOAD);
});

test("every permitted message kind has an accepted synthetic envelope", () => {
  const kinds = new Set(SYNTHETIC_ACCEPTED_BY_KIND.map((e) => e.kind));
  assert.equal(kinds.size, 7);
  for (let i = 0; i < SYNTHETIC_ACCEPTED_BY_KIND.length; i++) {
    const ctx: BridgeValidationContext = {
      ...FIXTURE_CONTEXT,
      lastAcceptedSequence: SYNTHETIC_ACCEPTED_BY_KIND[i].sequence - 1,
    };
    const result = validateBridgeEnvelope(SYNTHETIC_ACCEPTED_BY_KIND[i], ctx);
    assert.equal(result.ok, true, `kind ${SYNTHETIC_ACCEPTED_BY_KIND[i].kind} should be accepted`);
  }
});

test("each required rejection code is produced by a minimal hand-built envelope", () => {
  const cases: { code: BridgeRejectionCode; mutate: (e: BridgeEnvelope) => void }[] = [
    { code: "unknown_kind", mutate: (e) => { (e as unknown as Record<string, unknown>).kind = "host:grant-self"; } },
    { code: "version_mismatch", mutate: (e) => { e.protocolVersion = 2; } },
    { code: "session_mismatch", mutate: (e) => { e.sessionId = "other"; } },
    { code: "origin_mismatch", mutate: (e) => { e.expectedOriginId = "other"; } },
    { code: "sequence_replay", mutate: (e) => { e.sequence = 7; } },
    { code: "sequence_regression", mutate: (e) => { e.sequence = 1; } },
    { code: "declared_size_over_cap", mutate: (e) => { e.declaredPayloadBytes = 70000; e.payload = "z".repeat(70000); } },
    { code: "actual_size_over_cap", mutate: (e) => { e.payload = { blob: "x".repeat(70000) }; e.declaredPayloadBytes = bytesOf(e.payload); } },
    { code: "declared_size_mismatch", mutate: (e) => { e.declaredPayloadBytes = 3; } },
    { code: "malformed_shape", mutate: (e) => { (e as unknown as Record<string, unknown>).sequence = "eight"; } },
    { code: "missing_budget", mutate: (e) => { delete (e as unknown as Record<string, unknown>).budget; } },
    { code: "non_finite_budget", mutate: (e) => { e.budget.memoryBytes = Number.NaN; } },
    { code: "non_positive_or_non_integer_budget", mutate: (e) => { e.budget.messageCount = -1; } },
  ];
  for (const c of cases) {
    const env = baseEnvelope();
    c.mutate(env);
    const result = validateBridgeEnvelope(env, acceptedCtx());
    assert.equal(result.ok, false, `${c.code} should be rejected`);
    assert.ok(!result.ok);
    assert.equal(result.code, c.code);
  }
});

test("replay and regression are distinct codes", () => {
  const replayEnv = baseEnvelope();
  replayEnv.sequence = 7;
  const replay = validateBridgeEnvelope(replayEnv, acceptedCtx());
  const regEnv = baseEnvelope();
  regEnv.sequence = 2;
  const reg = validateBridgeEnvelope(regEnv, acceptedCtx());
  assert.ok(!replay.ok && !reg.ok);
  assert.equal(replay.code, "sequence_replay");
  assert.equal(reg.code, "sequence_regression");
  assert.notEqual(replay.code, reg.code);
});

test("over-cap declared length and over-cap actual payload are distinct codes", () => {
  const declaredEnv = baseEnvelope();
  declaredEnv.payload = "z".repeat(70000);
  declaredEnv.declaredPayloadBytes = 70001; // declared over cap, actual also over cap
  const declared = validateBridgeEnvelope(declaredEnv, acceptedCtx());
  assert.ok(!declared.ok);
  assert.equal(declared.code, "actual_size_over_cap"); // actual check fires first

  // Declared over cap while actual is under cap must yield declared_size_over_cap.
  const declaredOnly = baseEnvelope();
  declaredOnly.payload = "small";
  declaredOnly.declaredPayloadBytes = DEFAULT_MAX_PAYLOAD_BYTES + 1;
  const declaredOnlyResult = validateBridgeEnvelope(declaredOnly, acceptedCtx());
  assert.ok(!declaredOnlyResult.ok);
  assert.equal(declaredOnlyResult.code, "declared_size_over_cap");

  const actualEnv = baseEnvelope();
  actualEnv.payload = { blob: "x".repeat(70000) };
  actualEnv.declaredPayloadBytes = bytesOf(actualEnv.payload);
  const actual = validateBridgeEnvelope(actualEnv, acceptedCtx());
  assert.ok(!actual.ok);
  assert.equal(actual.code, "actual_size_over_cap");
  assert.notEqual(declaredOnlyResult.code, actual.code);
});

test("every synthetic fixture validates without throwing and matches its labeled expectation", () => {
  for (const fixture of SYNTHETIC_FIXTURES) {
    assert.equal(fixture.synthetic, true);
    let result: ReturnType<typeof validateBridgeEnvelope>;
    assert.doesNotThrow(() => {
      result = validateBridgeEnvelope(fixture.envelope, fixture.context);
    }, `fixture ${fixture.label} must not throw`);
    assert.ok(result!, `fixture ${fixture.label} must produce a result`);
    if (fixture.expectedRejection === null) {
      assert.equal(result!.ok, true, `fixture ${fixture.label} expected acceptance`);
    } else {
      assert.equal(result!.ok, false, `fixture ${fixture.label} expected rejection`);
      assert.ok(!result!.ok);
      assert.equal(result!.code, fixture.expectedRejection, `fixture ${fixture.label} code mismatch`);
    }
  }
});

test("hostile inputs never cause a throw", () => {
  const hostile: unknown[] = [
    null,
    undefined,
    0,
    1,
    "envelope",
    true,
    [],
    [1, 2, 3],
    Object.create(null),
    (() => {
      const o: Record<string, unknown> = {};
      Object.defineProperty(o, "kind", { get() { throw new Error("boom"); } });
      return o;
    })(),
    (() => {
      const circular: Record<string, unknown> = { kind: "host:cancel" };
      circular.self = circular;
      return circular;
    })(),
    { kind: "host:cancel", protocolVersion: PROTOCOL_VERSION, sessionId: "s", expectedOriginId: "o", sequence: 1, declaredPayloadBytes: 0, budget: { cpuMs: 1, memoryBytes: 1, messageCount: 1 }, payload: { __proto__: { x: 1 }, a: 1 } },
  ];
  for (const h of hostile) {
    let r: ReturnType<typeof validateBridgeEnvelope> | undefined;
    assert.doesNotThrow(() => {
      r = validateBridgeEnvelope(h, acceptedCtx());
    });
    assert.ok(r && typeof r.ok === "boolean");
  }
});

test("fixtures contain no credential-shaped or personal keys", () => {
  const serialized = JSON.stringify(SYNTHETIC_FIXTURES);
  const banned = ["password", "token", "cookie", "credential", "apiKey", "api_key", "secret", "authorization"];
  for (const word of banned) {
    assert.equal(serialized.toLowerCase().includes(word), false, `fixtures must not contain "${word}"`);
  }
});
