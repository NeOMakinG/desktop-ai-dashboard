// Pure contract fixture for the F05 e2e regression suite.
// No runtime, native, OS, or UI behavior. Parent F05 acceptance remains open.
export const STEP_MANIFEST = Object.freeze([
  Object.freeze({ id: 'build', surface: 'build', description: 'Application build completes' }),
  Object.freeze({ id: 'cold-launch', surface: 'cold-launch', description: 'Cold launch of the native app' }),
  Object.freeze({ id: 'chat-surface', surface: 'chat', description: 'Chat surface exercised' }),
  Object.freeze({ id: 'services-surface', surface: 'services', description: 'Services surface exercised' }),
  Object.freeze({ id: 'browser-surface', surface: 'browser', description: 'Browser surface exercised' }),
  Object.freeze({ id: 'schedules-surface', surface: 'schedules', description: 'Schedules surface exercised' }),
  Object.freeze({ id: 'screenshot-capture', surface: 'screenshots', description: 'Screenshot capture performed' }),
  Object.freeze({ id: 'summary-emission', surface: 'summary', description: 'Summary emitted' })
]);

export const STEP_IDS = Object.freeze(STEP_MANIFEST.map((s) => s.id));
export const RESULT_STATUSES = Object.freeze(['pass', 'fail', 'skipped']);
export const BLOCKER_MIN_REASON_LENGTH = 16;

const isNonEmptyString = (v) => typeof v === 'string' && v.trim().length > 0;

function fail(code, message, field) {
  const err = new Error(`${code}: ${message}`);
  err.code = code;
  if (field) err.field = field;
  return err;
}

export function validateStepResult(result) {
  if (result === null || typeof result !== 'object' || Array.isArray(result)) {
    throw fail('RESULT_NOT_OBJECT', 'step result must be a plain object');
  }
  const { stepId, status, sourceRevision, platform, screenshotReference } = result;
  if (!isNonEmptyString(stepId)) {
    throw fail('RESULT_MISSING_STEP_ID', 'stepId is required; a result claiming a surface was exercised must be tied to a step ID', 'stepId');
  }
  if (!STEP_IDS.includes(stepId)) {
    throw fail('RESULT_UNKNOWN_STEP_ID', `unknown stepId "${stepId}"`, 'stepId');
  }
  if (!RESULT_STATUSES.includes(status)) {
    throw fail('RESULT_INVALID_STATUS', `status must be one of ${RESULT_STATUSES.join(', ')}`, 'status');
  }
  if (!isNonEmptyString(sourceRevision)) {
    throw fail('RESULT_MISSING_SOURCE_REVISION', 'sourceRevision must be a non-empty string', 'sourceRevision');
  }
  if (!isNonEmptyString(platform)) {
    throw fail('RESULT_MISSING_PLATFORM', 'platform must be a non-empty string', 'platform');
  }
  if (stepId === 'screenshot-capture' && status === 'pass' && !isNonEmptyString(screenshotReference)) {
    throw fail('RESULT_PASS_MISSING_SCREENSHOT', 'a pass on screenshot-capture requires a non-empty screenshotReference', 'screenshotReference');
  }
  return Object.freeze({ stepId, status, sourceRevision, platform, screenshotReference: screenshotReference ?? null });
}

export function validateBlocker(blocker) {
  if (blocker === null || typeof blocker !== 'object' || Array.isArray(blocker)) {
    throw fail('BLOCKER_NOT_OBJECT', 'blocker must be a plain object');
  }
  const { stepId, reason } = blocker;
  if (!isNonEmptyString(stepId)) {
    throw fail('BLOCKER_MISSING_STEP_ID', 'blocker must be bound to a step ID; blanket suite-level blockers cannot be expressed', 'stepId');
  }
  if (!STEP_IDS.includes(stepId)) {
    throw fail('BLOCKER_UNKNOWN_STEP_ID', `unknown stepId "${stepId}"`, 'stepId');
  }
  if (typeof reason !== 'string' || reason.trim().length < BLOCKER_MIN_REASON_LENGTH) {
    throw fail('BLOCKER_REASON_TOO_SHORT', `reason must be a machine-readable string of at least ${BLOCKER_MIN_REASON_LENGTH} characters`, 'reason');
  }
  return Object.freeze({ stepId, reason: reason.trim() });
}

// Aggregation precedence (documented choice):
// - Any fail result or any blocker forces FAIL.
// - A missing step result forces FAIL (missing coverage is a failure, not a pass).
// - Skipped steps never force FAIL and never allow a blocker to hide; they are enumerated.
// - Duplicate results for the same step are rejected (ambiguous evidence).
export function aggregateSummary(results, blockers = []) {
  if (!Array.isArray(results)) throw fail('SUMMARY_RESULTS_NOT_ARRAY', 'results must be an array');
  if (!Array.isArray(blockers)) throw fail('SUMMARY_BLOCKERS_NOT_ARRAY', 'blockers must be an array');
  const byStep = new Map();
  for (const r of results) {
    const v = validateStepResult(r);
    if (byStep.has(v.stepId)) {
      throw fail('RESULT_DUPLICATE_STEP', `duplicate result for step "${v.stepId}"`, 'stepId');
    }
    byStep.set(v.stepId, v);
  }
  const validatedBlockers = blockers.map(validateBlocker);
  const failedStepIds = [];
  const blockedStepIds = [];
  const skippedStepIds = [];
  const missingStepIds = [];
  for (const id of STEP_IDS) {
    const r = byStep.get(id);
    if (!r) { missingStepIds.push(id); continue; }
    if (r.status === 'fail') failedStepIds.push(id);
    if (r.status === 'skipped') skippedStepIds.push(id);
  }
  for (const b of validatedBlockers) {
    if (!blockedStepIds.includes(b.stepId)) blockedStepIds.push(b.stepId);
  }
  const verdict = failedStepIds.length === 0 && blockedStepIds.length === 0 && missingStepIds.length === 0 ? 'PASS' : 'FAIL';
  return Object.freeze({
    verdict,
    failedStepIds: Object.freeze(failedStepIds),
    blockedStepIds: Object.freeze(blockedStepIds),
    skippedStepIds: Object.freeze(skippedStepIds),
    missingStepIds: Object.freeze(missingStepIds)
  });
}

export function makePassAllResults() {
  return STEP_IDS.map((id) => ({
    stepId: id,
    status: 'pass',
    sourceRevision: 'synthetic-rev-001',
    platform: 'synthetic-test-host',
    screenshotReference: id === 'screenshot-capture' ? 'synthetic://shots/cold-launch.png' : undefined
  }));
}
