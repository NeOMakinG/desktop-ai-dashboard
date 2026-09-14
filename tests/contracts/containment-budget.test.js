'use strict';

/**
 * Tests for src/contracts/containment-budget.js (task
 * F04-generated-ui-containment-fixture-27e1dfed).
 *
 * node:test / node:assert only, zero installed dependencies, Node v22.22.1.
 * Run: node --test tests/contracts/containment-budget.test.js
 */

import test from 'node:test';
import assert from 'node:assert/strict';

import {
  BUDGET_SHAPE,
  BUDGET_FIELDS,
  DEFAULT_BUDGET,
  HARD_MAXIMA,
  DENIED_CAPABILITIES,
  DENIAL_REASONS,
  validateBudget,
  normalizeBudget,
  requestCapability,
  SYNTHETIC_BUDGET_FIXTURES,
  SYNTHETIC_CAPABILITY_FIXTURES,
} from '../../src/contracts/containment-budget.js';

test('exports budget shape, defaults, and hard maxima for all five fields', () => {
  assert.deepEqual(
    [...BUDGET_FIELDS].sort(),
    ['cancellationDeadlineMs', 'cpuMs', 'instructionCap', 'memoryBytes', 'wallClockMs']
  );
  for (const field of BUDGET_FIELDS) {
    assert.equal(BUDGET_SHAPE[field].type, 'number');
    assert.ok(BUDGET_SHAPE[field].default <= BUDGET_SHAPE[field].maximum);
    assert.equal(DEFAULT_BUDGET[field], BUDGET_SHAPE[field].default);
    assert.equal(HARD_MAXIMA[field], BUDGET_SHAPE[field].maximum);
  }
  assert.ok(Object.isFrozen(BUDGET_SHAPE));
  assert.ok(Object.isFrozen(DEFAULT_BUDGET));
  assert.ok(Object.isFrozen(HARD_MAXIMA));
});

test('validateBudget accepts a well-formed synthetic fixture and freezes output', () => {
  const result = validateBudget(SYNTHETIC_BUDGET_FIXTURES.wellFormed);
  assert.equal(result.ok, true);
  assert.deepEqual(
    { ...result.budget },
    {
      wallClockMs: 1500,
      cpuMs: 800,
      memoryBytes: 16777216,
      instructionCap: 250000,
      cancellationDeadlineMs: 1800,
    }
  );
  assert.ok(Object.isFrozen(result.budget));
});

test('validateBudget rejects negative fields with BUDGET_NEGATIVE', () => {
  const result = validateBudget(SYNTHETIC_BUDGET_FIXTURES.negative);
  assert.equal(result.ok, false);
  assert.equal(result.errors.length, 1);
  assert.equal(result.errors[0].field, 'wallClockMs');
  assert.equal(result.errors[0].code, 'BUDGET_NEGATIVE');
});

test('validateBudget rejects non-numeric fields with BUDGET_NOT_FINITE_NUMBER', () => {
  const result = validateBudget(SYNTHETIC_BUDGET_FIXTURES.nonNumeric);
  assert.equal(result.ok, false);
  assert.equal(result.errors[0].field, 'wallClockMs');
  assert.equal(result.errors[0].code, 'BUDGET_NOT_FINITE_NUMBER');

  const nanInput = { ...SYNTHETIC_BUDGET_FIXTURES.wellFormed, cpuMs: Number.NaN };
  assert.equal(validateBudget(nanInput).errors[0].code, 'BUDGET_NOT_FINITE_NUMBER');
});

test('validateBudget rejects missing fields with BUDGET_MISSING_FIELD', () => {
  const result = validateBudget(SYNTHETIC_BUDGET_FIXTURES.missingField);
  assert.equal(result.ok, false);
  assert.equal(result.errors[0].field, 'cancellationDeadlineMs');
  assert.equal(result.errors[0].code, 'BUDGET_MISSING_FIELD');
});

test('validateBudget rejects non-object input with BUDGET_NOT_OBJECT', () => {
  for (const bad of [null, undefined, 42, 'budget', [1, 2]]) {
    const result = validateBudget(bad);
    assert.equal(result.ok, false);
    assert.equal(result.errors[0].code, 'BUDGET_NOT_OBJECT');
  }
});

test('validateBudget clamps over-maximum fields to hard maxima (documented clamp rule)', () => {
  const result = validateBudget(SYNTHETIC_BUDGET_FIXTURES.overMaximum);
  assert.equal(result.ok, true);
  assert.equal(result.budget.wallClockMs, HARD_MAXIMA.wallClockMs);
  assert.equal(result.budget.memoryBytes, HARD_MAXIMA.memoryBytes);
  assert.equal(result.budget.instructionCap, HARD_MAXIMA.instructionCap);
  assert.equal(result.budget.cancellationDeadlineMs, HARD_MAXIMA.cancellationDeadlineMs);
  assert.equal(result.budget.cpuMs, HARD_MAXIMA.cpuMs);
});

test('normalized output is immutable and unknown extra fields are dropped', () => {
  const budget = normalizeBudget({
    ...SYNTHETIC_BUDGET_FIXTURES.wellFormed,
    extraField: 'dropped',
  });
  assert.ok(Object.isFrozen(budget));
  assert.equal('extraField' in budget, false);
  assert.throws(() => { budget.wallClockMs = 0; }, TypeError);
});

test('normalizeBudget throws a descriptive error on invalid input', () => {
  assert.throws(() => normalizeBudget(SYNTHETIC_BUDGET_FIXTURES.negative), /BUDGET_NEGATIVE/);
});

test('every denied capability maps to a distinct typed denial reason', () => {
  const expectedCodes = {
    'ambient-network': 'DENIED_AMBIENT_NETWORK',
    'native-ipc': 'DENIED_NATIVE_IPC',
    'filesystem': 'DENIED_FILESYSTEM',
    'credentials': 'DENIED_CREDENTIALS',
    'dependency-installation': 'DENIED_DEPENDENCY_INSTALLATION',
    'device-enrollment': 'DENIED_DEVICE_ENROLLMENT',
  };
  const seen = new Set();
  for (const [capability, reason] of Object.entries(DENIED_CAPABILITIES)) {
    assert.equal(reason.code, expectedCodes[capability]);
    assert.equal(reason.capability, capability);
    assert.ok(reason.detail.length > 0);
    assert.ok(Object.isFrozen(reason));
    assert.equal(seen.has(reason.code), false, 'denial codes must be distinct');
    seen.add(reason.code);
  }
  assert.equal(Object.keys(DENIED_CAPABILITIES).length, 6);
  assert.ok(Object.isFrozen(DENIED_CAPABILITIES));
  assert.ok(Object.isFrozen(DENIAL_REASONS));
});

test('requesting any denied capability yields a denial, never a silent allowance', () => {
  for (const capability of SYNTHETIC_CAPABILITY_FIXTURES.deniedRequests) {
    const decision = requestCapability(capability);
    assert.equal(decision.allowed, false);
    assert.equal(decision.reason.code, DENIED_CAPABILITIES[capability].code);
    assert.ok(Object.isFrozen(decision));
  }
});

test('unknown capabilities are denied by default with a typed reason', () => {
  const decision = requestCapability(SYNTHETIC_CAPABILITY_FIXTURES.unknownRequest);
  assert.equal(decision.allowed, false);
  assert.equal(decision.reason.code, 'DENIED_UNKNOWN_CAPABILITY');
  assert.equal(requestCapability(undefined).reason.code, 'DENIED_UNKNOWN_CAPABILITY');
});

test('fixtures are labeled synthetic with no real-looking identifiers', () => {
  assert.equal(SYNTHETIC_BUDGET_FIXTURES.synthetic, true);
  assert.equal(SYNTHETIC_BUDGET_FIXTURES.wellFormed.synthetic, true);
  assert.equal(SYNTHETIC_CAPABILITY_FIXTURES.synthetic, true);
  assert.match(SYNTHETIC_BUDGET_FIXTURES.label, /synthetic/);
  assert.match(SYNTHETIC_CAPABILITY_FIXTURES.label, /synthetic/);
});

test('host-enforced-by-other-work: contract validation alone is not containment evidence (parent F04 remains open)', () => {
  // This test name records the boundary required by the task: this module is
  // a pure contract. Enforcement (watchdog, egress-channel blocking, host
  // survival e2e) is host-enforced-by-other-work; a green validateBudget or
  // denial result here is NOT containment evidence, and parent F04 runtime
  // acceptance remains OPEN.
  const budget = normalizeBudget(SYNTHETIC_BUDGET_FIXTURES.wellFormed);
  assert.ok(Object.isFrozen(budget));
  assert.equal(requestCapability('ambient-network').allowed, false);
});
