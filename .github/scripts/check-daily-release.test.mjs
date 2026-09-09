import assert from 'node:assert/strict';
import test from 'node:test';
import { isDailyReleaseDue } from './check-daily-release.mjs';

const run = { id: 12, head_sha: 'a'.repeat(40), head_branch: 'main', event: 'schedule', status: 'completed', conclusion: 'success' };
const health = { name: 'Production Railway health gate', run_id: 12, head_sha: run.head_sha, status: 'completed', conclusion: 'success', completed_at: '2026-09-09T14:20:00Z' };
const now = new Date('2026-09-09T14:38:00Z');
const requestFor = (runs, jobs) => async (path) => path.includes('/workflows/') ? { workflow_runs: runs } : { jobs };

test('manual and PR runs bypass the clock without querying release history', async () => {
  for (const event of ['workflow_dispatch', 'pull_request']) {
    assert.equal(await isDailyReleaseDue({ event, now, request: () => assert.fail('no query') }), true);
  }
});

test('Chicago 9 AM follows daylight saving time', async () => {
  for (const [timestamp, expected] of [
    ['2026-09-09T13:59:00Z', false], ['2026-09-09T14:00:00Z', true],
    ['2026-01-09T14:59:00Z', false], ['2026-01-09T15:00:00Z', true],
    ['2026-03-08T13:59:00Z', false], ['2026-03-08T14:00:00Z', true],
    ['2026-11-01T14:59:00Z', false], ['2026-11-01T15:00:00Z', true],
  ]) {
    assert.equal(await isDailyReleaseDue({ event: 'schedule', now: new Date(timestamp), request: requestFor([], []) }), expected, timestamp);
  }
});

test('missed polls remain eligible later in the same day', async () => {
  assert.equal(await isDailyReleaseDue({ event: 'schedule', now: new Date('2026-09-09T18:38:00Z'), request: requestFor([], []) }), true);
});

test('only a successful production health gate prevents a second daily release', async () => {
  for (const [runs, jobs, expected] of [
    [[run], [health], false],
    [[{ ...run, event: 'workflow_dispatch' }], [health], false],
    [[{ ...run, event: 'pull_request' }], [health], true],
    [[{ ...run, conclusion: 'failure' }], [health], true],
    [[run], [{ ...health, conclusion: 'failure' }], true],
    [[run], [{ ...health, conclusion: 'skipped' }], true],
    [[run], [{ ...health, head_sha: 'b'.repeat(40) }], true],
    [[run], [{ ...health, name: 'Check daily release time' }], true],
    [[run], [{ ...health, completed_at: '2026-09-09T03:00:00Z' }], true],
    [[run], [], true],
  ]) {
    assert.equal(await isDailyReleaseDue({ event: 'schedule', now, request: requestFor(runs, jobs) }), expected);
  }
});

test('a successful release becomes eligible on the next Chicago day', async () => {
  assert.equal(await isDailyReleaseDue({ event: 'schedule', now: new Date('2026-09-10T14:08:00Z'), request: requestFor([run], [health]) }), true);
});

test('pagination finds successful health evidence after no-op polls', async () => {
  const paths = [];
  const request = async (path) => {
    paths.push(path);
    if (path.includes('/workflows/')) return { workflow_runs: path.endsWith('page=1') ? Array(100).fill({ ...run, event: 'pull_request' }) : [run] };
    return { jobs: path.endsWith('page=1') ? Array(100).fill({ ...health, name: 'Other' }) : [health] };
  };
  assert.equal(await isDailyReleaseDue({ event: 'schedule', now, request }), false);
  assert.equal(paths.length, 4);
});

test('unreadable release history fails closed', async () => {
  await assert.rejects(isDailyReleaseDue({ event: 'schedule', now, request: async () => ({}) }), /Cannot read/);
  await assert.rejects(isDailyReleaseDue({ event: 'schedule', now, request: async () => { throw new Error('API unavailable'); } }), /API unavailable/);
});
