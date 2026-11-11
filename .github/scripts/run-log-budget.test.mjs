import assert from 'node:assert/strict';
import { mock, test } from 'node:test';
import { ISSUE_TITLE, judgeRunLogBudgets, reportRunLogBudget } from './run-log-budget.mjs';

const row = (trigger, samples, p75Ms, stored = samples) => ({ trigger, samples, p75Ms, stored });

test('budgets are judged only with twenty samples and breach above the trigger p75 or below 95% stored', () => {
  assert.deepEqual(judgeRunLogBudgets([
    row('job_click', 20, 401),
    row('list_click', 19, 5000, 0),
    row('page_load', 40, 2000),
  ]), {
    judged: 3,
    breaches: [
      { budget: 'p75 `job_click` at most 400 ms', value: '401 ms', samples: 20, breached: true },
      { budget: 'at least 95% served from stored logs', value: '75.9%', samples: 79, breached: true },
    ],
  });
  assert.deepEqual(judgeRunLogBudgets([row('job_click', 20, 400, 19), row('page_load', 19, 9000)]), { judged: 2, breaches: [] });
  assert.deepEqual(judgeRunLogBudgets([row('job_click', 19, 9000, 0)]), { judged: 0, breaches: [] });
});

function fakeGitHub(openIssues) {
  const create = mock.fn(async () => {});
  const update = mock.fn(async () => {});
  const createComment = mock.fn(async () => {});
  return {
    github: { rest: { issues: { listForRepo: 'issues', create, update, createComment } }, paginate: async () => openIssues },
    create, update, createComment,
  };
}

const env = { POSTHOG_PERSONAL_API_KEY: 'fixture-only', POSTHOG_PROJECT_ID: '123' };
const runUrl = 'https://github.com/o/r/actions/runs/1';
const answer = rows => async () => ({
  columns: ['trigger', 'samples', 'p75_ms', 'stored'],
  results: rows.map(item => [item.trigger, String(item.samples), item.p75Ms, item.stored]),
});

test('a breach opens one issue or rewrites the open one, a passing week closes it, and an unjudged week leaves it', async () => {
  const slow = answer([row('job_click', 30, 650)]);
  const fresh = fakeGitHub([{ number: 3, title: 'Unrelated' }]);
  await reportRunLogBudget({ github: fresh.github, owner: 'o', repo: 'r', runUrl, env, request: slow });
  assert.equal(fresh.create.mock.callCount(), 1);
  assert.equal(fresh.create.mock.calls[0].arguments[0].title, ISSUE_TITLE);
  assert.match(fresh.create.mock.calls[0].arguments[0].body, /\| p75 `job_click` at most 400 ms \| 650 ms \| 30 \|/);

  const recurring = fakeGitHub([{ number: 7, title: ISSUE_TITLE }]);
  await reportRunLogBudget({ github: recurring.github, owner: 'o', repo: 'r', runUrl, env, request: slow });
  assert.equal(recurring.create.mock.callCount(), 0);
  assert.equal(recurring.update.mock.callCount(), 1);
  assert.equal(recurring.update.mock.calls[0].arguments[0].issue_number, 7);
  assert.match(recurring.update.mock.calls[0].arguments[0].body, /650 ms/);

  const recovered = fakeGitHub([{ number: 7, title: ISSUE_TITLE }]);
  await reportRunLogBudget({ github: recovered.github, owner: 'o', repo: 'r', runUrl, env, request: answer([row('job_click', 30, 300)]) });
  assert.equal(recovered.update.mock.callCount(), 1);
  assert.equal(recovered.update.mock.calls[0].arguments[0].state, 'closed');

  const quiet = fakeGitHub([{ number: 7, title: ISSUE_TITLE }]);
  assert.equal(await reportRunLogBudget({ github: quiet.github, owner: 'o', repo: 'r', runUrl, env, request: answer([row('job_click', 5, 9000)]) }), null);
  assert.equal(quiet.update.mock.callCount() + quiet.createComment.mock.callCount(), 0);
});

test('a missing PostHog secret fails before any query names the secret', async () => {
  const request = mock.fn();
  for (const missing of ['POSTHOG_PERSONAL_API_KEY', 'POSTHOG_PROJECT_ID']) {
    await assert.rejects(
      reportRunLogBudget({ github: fakeGitHub([]).github, owner: 'o', repo: 'r', runUrl, env: { ...env, [missing]: '' }, request }),
      { message: `${missing} is not set` },
    );
  }
  assert.equal(request.mock.callCount(), 0);
});
