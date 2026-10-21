import assert from 'node:assert/strict';
import test from 'node:test';
import { RailwayRequestError, retryRailway } from './railway-retry.mjs';

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

test('reports the redacted Railway error on each retry and the final failure', () => {
  const reports = [];
  assert.throws(() => retryRailway(() => {
    throw new RailwayRequestError('ConfigureRegistry', 'BAD_USER_INPUT: Deploys have been paused temporarily');
  }, { pause: () => {}, report: (message) => reports.push(message) }),
  { message: 'Railway request failed after 3 attempts (BAD_USER_INPUT: Deploys have been paused temporarily).' });
  assert.deepEqual(reports, [
    'Railway request failed (BAD_USER_INPUT: Deploys have been paused temporarily); retrying (1/3).',
    'Railway request failed (BAD_USER_INPUT: Deploys have been paused temporarily); retrying (2/3).',
  ]);
});
