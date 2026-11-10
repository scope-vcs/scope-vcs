import assert from 'node:assert/strict';
import { mock, test } from 'node:test';
import { ISSUE_TITLE, reportCiDuration, slowJobs } from './ci-duration.mjs';

const start = Date.parse('2026-11-09T10:00:00Z');
const run = (index, jobs) => ({
  url: `https://github.com/scope-vcs/scope-vcs/actions/runs/${index}`,
  startedAt: new Date(start).toISOString(),
  jobs: Object.entries(jobs).map(([name, seconds]) => ({
    name, conclusion: seconds === null ? 'skipped' : 'success',
    completedAt: new Date(start + (seconds ?? 0) * 1000).toISOString(),
  })),
});

test('a job is slow only when the median of its last ten successful runs is over budget', () => {
  const runs = Array.from({ length: 12 }, (_, index) => run(index, {
    'Browser smoke': index < 10 ? 460 + index : 100,
    'Windows native checks': index === 0 ? 900 : 300,
    'Runner image': index < 9 ? 600 : null,
  }));
  const jobs = slowJobs(runs);
  assert.deepEqual(jobs.map(job => [job.name, job.medianSeconds]), [['Browser smoke', 464.5]]);
  assert.equal(jobs[0].runs.length, 10);
});

const pullRequestRun = (id, seconds) => ({
  id, run_attempt: 1, html_url: `https://github.com/r/${id}`, run_started_at: new Date(start).toISOString(),
  seconds,
});

function fakeGitHub(openIssues, seconds) {
  const runs = Array.from({ length: 10 }, (_, index) => pullRequestRun(index, seconds));
  const create = mock.fn(async () => {});
  const createComment = mock.fn(async () => {});
  const github = {
    rest: {
      actions: {
        listWorkflowRuns: async () => ({ data: { workflow_runs: [...runs, { ...runs[0], id: 99, run_attempt: 2 }] } }),
        listJobsForWorkflowRun: 'jobs',
      },
      issues: { listForRepo: 'issues', create, createComment },
    },
    paginate: async (method, { run_id: runId }) => (method === 'issues' ? openIssues : [{
      name: 'Browser smoke', conclusion: 'success',
      completed_at: new Date(start + runs.find(item => item.id === runId).seconds * 1000).toISOString(),
    }]),
  };
  return { github, create, createComment };
}

test('a slow job comments on the open budget issue instead of opening another, and a fast run reports nothing', async () => {
  const runUrl = 'https://github.com/scope-vcs/scope-vcs/actions/runs/1';
  const fresh = fakeGitHub([{ number: 3, title: 'Unrelated' }], 500);
  await reportCiDuration({ github: fresh.github, owner: 'o', repo: 'r', runUrl });
  assert.equal(fresh.create.mock.callCount(), 1);
  assert.equal(fresh.create.mock.calls[0].arguments[0].title, ISSUE_TITLE);
  assert.match(fresh.create.mock.calls[0].arguments[0].body, /\| Browser smoke \| 8\.3 \|/);

  const recurring = fakeGitHub([{ number: 7, title: ISSUE_TITLE }], 500);
  await reportCiDuration({ github: recurring.github, owner: 'o', repo: 'r', runUrl });
  assert.equal(recurring.create.mock.callCount(), 0);
  assert.equal(recurring.createComment.mock.callCount(), 1);
  assert.equal(recurring.createComment.mock.calls[0].arguments[0].issue_number, 7);

  const fast = fakeGitHub([], 300);
  assert.equal(await reportCiDuration({ github: fast.github, owner: 'o', repo: 'r', runUrl }), null);
  assert.equal(fast.create.mock.callCount() + fast.createComment.mock.callCount(), 0);
});
