/**
 * Generated-UI containment execution-budget and capability-denial contract.
 *
 * Pure, dependency-free contract per ADR 0001 (generated UI is a product
 * capability, not host authority). Defines:
 *   - a frozen budget shape (wall-clock ms, CPU ms, memory bytes,
 *     instruction/iteration cap, cancellation deadline) with defaults and
 *     hard maxima,
 *   - a frozen set of capabilities that are ALWAYS denied in the contained
 *     runtime (ambient network, native/IPC, filesystem, credentials,
 *     dependency installation, device enrollment), each mapped to a typed
 *     denial reason, with deny-by-default for unknown capabilities,
 *   - validate/normalize functions implementing documented clamp-or-reject
 *     rules, producing a deep-frozen normalized budget.
 *
 * Pre-check note (task F04-generated-ui-containment-fixture-27e1dfed):
 * no equivalent budget/capability module was found in src/contracts/ or
 * src/domain/, so this file is new rather than an extension.
 *
 * IMPORTANT: this contract is host-enforced-by-other-work. Contract
 * validation alone is NOT containment evidence. The parent F04 runtime
 * acceptance (watchdog, egress-channel tests, host survival e2e) remains
 * OPEN and is not claimed by this module.
 *
 * Fixtures embedded here are labeled synthetic per ADR 0001's
 * gate-before-live-data rule; no real credentials, accounts, or endpoints.
 */

export const BUDGET_SHAPE = Object.freeze({
  wallClockMs: Object.freeze({ type: 'number', minimum: 0, default: 2000, maximum: 10000 }),
  cpuMs: Object.freeze({ type: 'number', minimum: 0, default: 1000, maximum: 5000 }),
  memoryBytes: Object.freeze({ type: 'number', minimum: 0, default: 32 * 1024 * 1024, maximum: 256 * 1024 * 1024 }),
  instructionCap: Object.freeze({ type: 'number', minimum: 0, default: 1_000_000, maximum: 50_000_000 }),
  cancellationDeadlineMs: Object.freeze({ type: 'number', minimum: 0, default: 2500, maximum: 12000 }),
});

export const BUDGET_FIELDS = Object.freeze(Object.keys(BUDGET_SHAPE));

export const DEFAULT_BUDGET = Object.freeze(
  Object.fromEntries(BUDGET_FIELDS.map((f) => [f, BUDGET_SHAPE[f].default]))
);

export const HARD_MAXIMA = Object.freeze(
  Object.fromEntries(BUDGET_FIELDS.map((f) => [f, BUDGET_SHAPE[f].maximum]))
);

/**
 * Typed denial reasons. Every capability in DENIED_CAPABILITIES maps to a
 * distinct typed reason; requesting any of them yields a denial, never a
 * silent allowance. Unknown capabilities are denied by default with
 * DENIED_UNKNOWN_CAPABILITY.
 */
export const DENIAL_REASONS = Object.freeze({
  AMBIENT_NETWORK: Object.freeze({
    code: 'DENIED_AMBIENT_NETWORK',
    capability: 'ambient-network',
    detail: 'Generated UI has no ambient network access; egress requires a host-validated bridge.',
  }),
  NATIVE_IPC: Object.freeze({
    code: 'DENIED_NATIVE_IPC',
    capability: 'native-ipc',
    detail: 'Native and IPC channels are unavailable to generated UI.',
  }),
  FILESYSTEM: Object.freeze({
    code: 'DENIED_FILESYSTEM',
    capability: 'filesystem',
    detail: 'No direct filesystem access; only host-approved data projections.',
  }),
  CREDENTIALS: Object.freeze({
    code: 'DENIED_CREDENTIALS',
    capability: 'credentials',
    detail: 'Credentials and connector tokens never cross into the contained runtime.',
  }),
  DEPENDENCY_INSTALLATION: Object.freeze({
    code: 'DENIED_DEPENDENCY_INSTALLATION',
    capability: 'dependency-installation',
    detail: 'Installing dependencies from generated UI is forbidden.',
  }),
  DEVICE_ENROLLMENT: Object.freeze({
    code: 'DENIED_DEVICE_ENROLLMENT',
    detail: 'Generated UI cannot enroll devices or mint authority.',
    capability: 'device-enrollment',
  }),
  UNKNOWN_CAPABILITY: Object.freeze({
    code: 'DENIED_UNKNOWN_CAPABILITY',
    capability: 'unknown',
    detail: 'Deny-by-default: capability not in the allowed set.',
  }),
});

export const DENIED_CAPABILITIES = Object.freeze({
  'ambient-network': DENIAL_REASONS.AMBIENT_NETWORK,
  'native-ipc': DENIAL_REASONS.NATIVE_IPC,
  'filesystem': DENIAL_REASONS.FILESYSTEM,
  'credentials': DENIAL_REASONS.CREDENTIALS,
  'dependency-installation': DENIAL_REASONS.DEPENDENCY_INSTALLATION,
  'device-enrollment': DENIAL_REASONS.DEVICE_ENROLLMENT,
});

function deepFreeze(value) {
  if (value && typeof value === 'object' && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const key of Object.keys(value)) deepFreeze(value[key]);
  }
  return value;
}

/**
 * validateBudget — documented clamp-or-reject rules:
 *   REJECT when: input is not an object; a known field is missing, not a
 *   finite number, or negative (below minimum).
 *   CLAMP when: a known field exceeds its hard maximum (clamped to maximum).
 *   IGNORE: unknown extra fields are dropped during normalization.
 * Returns { ok: true, budget } with the clamped, deep-frozen budget, or
 * { ok: false, errors: [{ field, code, message }] }.
 */
export function validateBudget(input) {
  if (input === null || typeof input !== 'object' || Array.isArray(input)) {
    return {
      ok: false,
      errors: [Object.freeze({
        field: '*',
        code: 'BUDGET_NOT_OBJECT',
        message: 'Budget must be a plain object of numeric fields.',
      })],
    };
  }

  const errors = [];
  const normalized = {};

  for (const field of BUDGET_FIELDS) {
    const spec = BUDGET_SHAPE[field];
    const raw = input[field];

    if (raw === undefined) {
      errors.push({ field, code: 'BUDGET_MISSING_FIELD', message: `Missing required field "${field}".` });
      continue;
    }
    if (typeof raw !== 'number' || !Number.isFinite(raw)) {
      errors.push({ field, code: 'BUDGET_NOT_FINITE_NUMBER', message: `Field "${field}" must be a finite number.` });
      continue;
    }
    if (raw < spec.minimum) {
      errors.push({ field, code: 'BUDGET_NEGATIVE', message: `Field "${field}" must be >= ${spec.minimum}.` });
      continue;
    }
    normalized[field] = Math.min(raw, spec.maximum);
  }

  if (errors.length > 0) {
    return { ok: false, errors: deepFreeze(errors) };
  }
  return { ok: true, budget: deepFreeze(normalized) };
}

/**
 * normalizeBudget — validate then return the deep-frozen budget, or throw
 * an Error listing the validation failures. Never returns a mutable result.
 */
export function normalizeBudget(input) {
  const result = validateBudget(input);
  if (!result.ok) {
    const summary = result.errors.map((e) => `${e.field}: ${e.code}`).join('; ');
    throw new Error(`Invalid containment budget: ${summary}`);
  }
  return result.budget;
}

/**
 * requestCapability — every capability in DENIED_CAPABILITIES yields
 * { allowed: false, reason: <typed denial> }; unknown capabilities are
 * denied by default. There is no input that produces allowed: true from
 * this contract; allowances belong to host-enforced other work.
 */
export function requestCapability(name) {
  const key = typeof name === 'string' ? name : 'unknown';
  const reason = DENIED_CAPABILITIES[key] ?? DENIAL_REASONS.UNKNOWN_CAPABILITY;
  return Object.freeze({ allowed: false, reason });
}

/**
 * Labeled synthetic fixtures (gate-before-live-data, ADR 0001).
 * No real credentials, account data, or live endpoints appear anywhere here.
 */
export const SYNTHETIC_BUDGET_FIXTURES = Object.freeze({
  synthetic: true,
  label: 'synthetic-containment-budget-fixtures',
  wellFormed: Object.freeze({
    synthetic: true,
    wallClockMs: 1500,
    cpuMs: 800,
    memoryBytes: 16777216,
    instructionCap: 250000,
    cancellationDeadlineMs: 1800,
  }),
  overMaximum: Object.freeze({
    synthetic: true,
    wallClockMs: 999999,
    cpuMs: 5000,
    memoryBytes: 68719476736,
    instructionCap: 999999999,
    cancellationDeadlineMs: 60000,
  }),
  negative: Object.freeze({
    synthetic: true,
    wallClockMs: -5,
    cpuMs: 100,
    memoryBytes: 1048576,
    instructionCap: 1000,
    cancellationDeadlineMs: 500,
  }),
  nonNumeric: Object.freeze({
    synthetic: true,
    wallClockMs: 'fast',
    cpuMs: 100,
    memoryBytes: 1048576,
    instructionCap: 1000,
    cancellationDeadlineMs: 500,
  }),
  missingField: Object.freeze({
    synthetic: true,
    wallClockMs: 1000,
    cpuMs: 100,
    memoryBytes: 1048576,
    instructionCap: 1000,
  }),
});

export const SYNTHETIC_CAPABILITY_FIXTURES = Object.freeze({
  synthetic: true,
  label: 'synthetic-capability-request-fixtures',
  deniedRequests: Object.freeze([
    'ambient-network',
    'native-ipc',
    'filesystem',
    'credentials',
    'dependency-installation',
    'device-enrollment',
  ]),
  unknownRequest: 'synthetic-unknown-capability',
});
