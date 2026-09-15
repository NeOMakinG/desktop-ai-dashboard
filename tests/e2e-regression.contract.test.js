import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  STEP_MANIFEST, STEP_IDS, RESULT_STATUSES, BLOCKER_MIN_REASON_LENGTH,
  validateStepResult, validateBlocker, aggregateSummary, makePassAllResults
} from '../src/contracts/e2e-regression.js';

const base = { stepId: 'build', status: 'pass', sourceRevision: 'rev-1', platform: 'test-host' };
const errCode = (fn) => { try { fn(); } catch (e) { return e.code; } return null; };

test('manifest is frozen with the eight stable step IDs', () => {
  assert.ok(Object.isFrozen(STEP_MANIFEST));
  for (const s of STEP_MANIFEST) assert.ok(Object.isFrozen(s));
  assert.deepEqual(STEP_IDS, [
    'build', 'cold-launch', 'chat-surface', 'services-surface',
    'browser-surface', 'schedules-surface', 'screenshot-capture', 'summary-emission'
  ]);
  assert.ok(Object.isFrozen(STEP_IDS));
  for (const s of STEP_MANIFEST) {
    assert.equal(typeof s.id, 'string');
    assert.equal(typeof s.surface, 'string');
    assert.equal(typeof s.description, 'string');
  }
});

test('validateStepResult accepts well-formed pass/fail/skipped results and freezes output', () => {
  for (const status of RESULT_STATUSES) {
    const v = validateStepResult({ ...base, status });
    assert.equal(v.status, status);
    assert.ok(Object.isFrozen(v));
  }
  assert.equal(validateStepResult({ ...base }).sourceRevision, 'rev-1');
  assert.equal(validateStepResult({ ...base }).platform, 'test-host');
});

test('validateStepResult rejects malformed results with specific codes', () => {
  assert.equal(errCode(() => validateStepResult(null)), 'RESULT_NOT_OBJECT');
  assert.equal(errCode(() => validateStepResult('x')), 'RESULT_NOT_OBJECT');
  assert.equal(errCode(() => validateStepResult({ status: 'pass', sourceRevision: 'r', platform: 'p' })), 'RESULT_MISSING_STEP_ID');
  assert.equal(errCode(() => validateStepResult({ ...base, stepId: 'nope' })), 'RESULT_UNKNOWN_STEP_ID');
  assert.equal(errCode(() => validateStepResult({ ...base, status: 'crashed' })), 'RESULT_INVALID_STATUS');
  assert.equal(errCode(() => validateStepResult({ ...base, sourceRevision: '' })), 'RESULT_MISSING_SOURCE_REVISION');
  assert.equal(errCode(() => validateStepResult({ ...base, sourceRevision: '   ' })), 'RESULT_MISSING_SOURCE_REVISION');
  assert.equal(errCode(() => validateStepResult({ ...base, platform: '' })), 'RESULT_MISSING_PLATFORM');
});

test('screenshot-capture pass requires a screenshot reference; other steps do not', () => {
  const shot = { ...base, stepId: 'screenshot-capture' };
  assert.equal(errCode(() => validateStepResult(shot)), 'RESULT_PASS_MISSING_SCREENSHOT');
  assert.equal(errCode(() => validateStepResult({ ...shot, screenshotReference: '  ' })), 'RESULT_PASS_MISSING_SCREENSHOT');
  assert.equal(validateStepResult({ ...shot, screenshotReference: 'synthetic://a.png' }).screenshotReference, 'synthetic://a.png');
  assert.equal(errCode(() => validateStepResult({ ...shot, status: 'fail' })), null);
  assert.equal(errCode(() => validateStepResult(base)), null);
});

test('blocker requires a precise reason bound to an existing step', () => {
  const good = { stepId: 'browser-surface', reason: 'chromium packaging unavailable on host' };
  assert.equal(validateBlocker(good).stepId, 'browser-surface');
  assert.ok(Object.isFrozen(validateBlocker(good)));
  assert.equal(errCode(() => validateBlocker(null)), 'BLOCKER_NOT_OBJECT');
  assert.equal(errCode(() => validateBlocker({ reason: 'x'.repeat(40) })), 'BLOCKER_MISSING_STEP_ID');
  assert.equal(errCode(() => validateBlocker({ ...good, stepId: 'suite' })), 'BLOCKER_UNKNOWN_STEP_ID');
  assert.equal(errCode(() => validateBlocker({ ...good, reason: '' })), 'BLOCKER_REASON_TOO_SHORT');
  assert.equal(errCode(() => validateBlocker({ ...good, reason: '   ' })), 'BLOCKER_REASON_TOO_SHORT');
  assert.equal(errCode(() => validateBlocker({ ...good, reason: 'too short' })), 'BLOCKER_REASON_TOO_SHORT');
  assert.ok(' '.repeat(BLOCKER_MIN_REASON_LENGTH).length >= BLOCKER_MIN_REASON_LENGTH);
});

test('all-pass summary yields a single PASS verdict (screenshot fixture non-empty)', () => {
  const results = makePassAllResults();
  const shot = results.find((r) => r.stepId === 'screenshot-capture');
  assert.ok(typeof shot.screenshotReference === 'string' && shot.screenshotReference.length > 0);
  const s = aggregateSummary(results, []);
  assert.equal(s.verdict, 'PASS');
  assert.deepEqual(s.failedStepIds, []);
  assert.deepEqual(s.blockedStepIds, []);
  assert.deepEqual(s.skippedStepIds, []);
  assert.deepEqual(s.missingStepIds, []);
  assert.ok(Object.isFrozen(s));
});

test('any fail or blocker yields FAIL enumerating every affected step', () => {
  const results = makePassAllResults().map((r) =>
    r.stepId === 'chat-surface' ? { ...r, status: 'fail' } : r
  );
  const blockers = [{ stepId: 'browser-surface', reason: 'chromium packaging unavailable on host' }];
  const s = aggregateSummary(results, blockers);
  assert.equal(s.verdict, 'FAIL');
  assert.deepEqual(s.failedStepIds, ['chat-surface']);
  assert.deepEqual(s.blockedStepIds, ['browser-surface']);
});

test('PASS is unreachable while any step is blocked, failed, or missing', () => {
  const blocked = aggregateSummary(makePassAllResults(), [{ stepId: 'build', reason: 'toolchain missing on runner host' }]);
  assert.equal(blocked.verdict, 'FAIL');
  const missing = aggregateSummary(makePassAllResults().slice(0, 7), []);
  assert.equal(missing.verdict, 'FAIL');
  assert.deepEqual(missing.missingStepIds, ['summary-emission']);
  const failed = aggregateSummary(makePassAllResults().map((r) => ({ ...r, status: 'fail' })), []);
  assert.equal(failed.verdict, 'FAIL');
  assert.equal(failed.failedStepIds.length, STEP_IDS.length);
});

test('skipped steps are enumerated and do not alone force FAIL', () => {
  const results = makePassAllResults().map((r) =>
    r.stepId === 'services-surface' ? { ...r, status: 'skipped' } : r
  );
  const s = aggregateSummary(results, []);
  assert.deepEqual(s.skippedStepIds, ['services-surface']);
  assert.equal(s.verdict, 'PASS');
});

test('aggregation rejects duplicates and invalid inputs', () => {
  assert.equal(errCode(() => aggregateSummary('nope')), 'SUMMARY_RESULTS_NOT_ARRAY');
  assert.equal(errCode(() => aggregateSummary([], 'nope')), 'SUMMARY_BLOCKERS_NOT_ARRAY');
  assert.equal(
    errCode(() => aggregateSummary([...makePassAllResults(), ...makePassAllResults()], [])),
    'RESULT_DUPLICATE_STEP'
  );
  assert.equal(
    errCode(() => aggregateSummary([{ stepId: 'build', status: 'weird', sourceRevision: 'r', platform: 'p' }], [])),
    'RESULT_INVALID_STATUS'
  );
  assert.equal(
    errCode(() => aggregateSummary([], [{ stepId: 'build', reason: 'no' }])),
    'BLOCKER_REASON_TOO_SHORT'
  );
});

test('contract-only fixture: no runtime acceptance claimed, parent F05 remains open', () => {
  const src = JSON.stringify(STEP_MANIFEST);
  assert.ok(!src.includes('http'));
  assert.equal(typeof validateStepResult, 'function');
  assert.equal(typeof validateBlocker, 'function');
  assert.equal(typeof aggregateSummary, 'function');
});
