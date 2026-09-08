import assert from 'node:assert/strict';
import test from 'node:test';
import { retryRailway } from './railway-retry.mjs';

test('bounds repeated provider failures and suppresses secret diagnostics', () => {
  let calls = 0;
  const pauses = [];
  const reports = [];
  assert.throws(() => retryRailway(() => {
    calls += 1;
    throw new Error('SECRET response body');
  }, { pause: (delay) => pauses.push(delay), report: (message) => reports.push(message) }),
  { message: 'Railway request failed after 3 attempts.' });
  assert.equal(calls, 3);
  assert.deepEqual(pauses, [2_000, 2_000]);
  assert.equal(reports.length, 2);
  assert.doesNotMatch(reports.join(''), /SECRET/);
});
