import assert from 'node:assert/strict';
import test, { mock } from 'node:test';
import { reportPullRequestChecks } from './report-pr-checks.mjs';

const sha = 'a'.repeat(40);
const pull = { state: 'open', merged: false, head: { sha } };
const run = { id: 123, event: 'pull_request', head_sha: sha, status: 'in_progress', conclusion: null, pull_requests: [] };

function scenario({ pulls = [pull], runs = [run], timeout = 90_000 } = {}) {
  let elapsed = 0;
  let pauses = 0;
  const messages = [];
  const paths = [];
  const timeouts = [];
  const queues = {
    'repos/o/r/pulls/7': [...pulls],
    'repos/o/r/actions/runs/123': [...runs],
  };
  return {
    messages, paths, timeouts,
    pauses: () => pauses,
    elapsed: () => elapsed,
    result: reportPullRequestChecks({
      repository: 'o/r', sha, pullRequest: 7, runId: 123, timeout,
      now: () => elapsed,
      pause: async milliseconds => { elapsed += milliseconds; pauses += 1; },
      report: message => messages.push(message),
      request: mock.fn(async (path, requestTimeout) => {
        paths.push(path);
        timeouts.push({ requestTimeout, remaining: timeout - elapsed });
        const queue = queues[path];
        if (!queue) throw new Error(`Unexpected GitHub API endpoint: ${path}`);
        const response = queue.length > 1 ? queue.shift() : queue[0];
        if (response instanceof Error) throw response;
        return response;
      }),
    }),
  };
}

test('reports success only after the pinned run completes, even without a PR association', async () => {
  const result = scenario({ runs: [run, { ...run, status: 'completed', conclusion: 'success' }] });
  await result.result;
  assert.equal(result.pauses(), 1);
  assert.match(result.messages.at(-1), /actions\/runs\/123: success/);
});

test('fails promptly when the PR head moves mid-wait and its old run loses association', async () => {
  const result = scenario({ pulls: [pull, { ...pull, head: { sha: 'b'.repeat(40) } }] });
  await assert.rejects(result.result, /superseded.*head=bbbb.*refusing to report success/);
  assert.equal(result.pauses(), 1);
  assert.equal(result.paths.filter(path => path.endsWith('/runs/123')).length, 1);
});

test('fails at its deadline for a run stuck in progress or repeated API failures', async () => {
  for (const response of [run, new Error('HTTP 502')]) {
    const result = scenario({ runs: [response], timeout: 45_000 });
    await assert.rejects(result.result, /Deadline exceeded.*runs\/123.*no successful result was confirmed/);
    assert.equal(result.elapsed(), 45_000);
    assert.equal(result.pauses(), 2);
    for (const { requestTimeout, remaining } of result.timeouts) {
      assert.ok(requestTimeout > 0 && requestTimeout <= Math.min(30_000, remaining));
    }
  }
});

test('fails promptly when the PR closes or merges during the wait', async () => {
  for (const merged of [false, true]) {
    const result = scenario({ pulls: [pull, { ...pull, state: 'closed', merged }] });
    await assert.rejects(result.result, /superseded.*state=closed/);
    assert.equal(result.pauses(), 1);
  }
});

test('retries transient errors reading either the PR or its pinned run', async () => {
  for (const endpoint of ['pulls', 'runs']) {
    const success = { ...run, status: 'completed', conclusion: 'success' };
    const result = scenario({
      pulls: endpoint === 'pulls' ? [new Error('HTTP 502'), pull] : [pull],
      runs: endpoint === 'runs' ? [new Error('HTTP 502'), success] : [success],
    });
    await result.result;
    assert.equal(result.pauses(), 1);
    assert.match(result.messages.join('\n'), /HTTP 502; retrying/);
    assert.match(result.messages.at(-1), /: success/);
  }
});

test('rejects unsuccessful conclusions and a run for another SHA or event', async () => {
  for (const conclusion of ['failure', 'cancelled', 'timed_out', 'skipped', 'neutral', 'action_required', null]) {
    const result = scenario({ runs: [{ ...run, status: 'completed', conclusion }] });
    await assert.rejects(result.result, /required checks failed/);
  }
  for (const change of [{ head_sha: 'b'.repeat(40) }, { event: 'push' }, { id: 456 }]) {
    await assert.rejects(scenario({ runs: [{ ...run, status: 'completed', conclusion: 'success', ...change }] }).result, /does not test pull request commit/);
  }
});
