import assert from 'node:assert/strict';
import { mock, test } from 'node:test';
import { ISSUE_TITLE, recentPullRequestRuns, reportCiDuration, slowJobs } from './ci-duration.mjs';

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
  head_repository: { full_name: 'o/r' }, seconds,
});

function fakeGitHub(openIssues, seconds) {
  const runs = Array.from({ length: 10 }, (_, index) => pullRequestRun(index, seconds));
  const create = mock.fn(async () => {});
  const createComment = mock.fn(async () => {});
  const github = {
    rest: {
      actions: {
        listWorkflowRuns: async () => ({ data: { workflow_runs: [
          ...runs, { ...runs[0], id: 99, run_attempt: 2 },
          ...runs.map(run => ({ ...run, id: run.id + 100, head_repository: { full_name: 'fork/r' } })),
        ] } }),
        listJobsForWorkflowRun: 'jobs',
      },
      issues: { listForRepo: 'issues', create, createComment },
      repos: { listCommits: async () => ({ data: [{ commit: { committer: { date: new Date(start - 1000).toISOString() } } }] }) },
    },
    paginate: async (method, { run_id: runId }) => (method === 'issues' ? openIssues : [{
      name: runId >= 100 ? '@everyone | [x](y)' : 'Browser smoke', conclusion: 'success',
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
  assert.match(fresh.create.mock.calls[0].arguments[0].body, /\| `Browser smoke` \| 8\.3 \|/);
  assert.doesNotMatch(fresh.create.mock.calls[0].arguments[0].body, /@everyone/);

  const recurring = fakeGitHub([{ number: 7, title: ISSUE_TITLE }], 500);
  await reportCiDuration({ github: recurring.github, owner: 'o', repo: 'r', runUrl });
  assert.equal(recurring.create.mock.callCount(), 0);
  assert.equal(recurring.createComment.mock.callCount(), 1);
  assert.equal(recurring.createComment.mock.calls[0].arguments[0].issue_number, 7);

  const fast = fakeGitHub([], 300);
  assert.equal(await reportCiDuration({ github: fast.github, owner: 'o', repo: 'r', runUrl }), null);
  assert.equal(fast.create.mock.callCount() + fast.createComment.mock.callCount(), 0);
});

test('older pages are read until every job has ten successful samples, stopping at the last workflow change or 200 runs', async () => {
  const history = async (rareEvery, sinceRuns = 1000) => {
    const listWorkflowRuns = mock.fn(async ({ page }) => ({ data: { workflow_runs: Array.from({ length: 50 }, (_, index) => ({
      id: (page - 1) * 50 + index, run_attempt: 1, html_url: 'u',
      run_started_at: new Date(start - ((page - 1) * 50 + index) * 60_000).toISOString(),
      head_repository: { full_name: 'o/r' },
    })) } }));
    const github = {
      rest: { actions: { listWorkflowRuns, listJobsForWorkflowRun: 'jobs' } },
      paginate: async (_, { run_id: id }) => [
        { name: 'Common', conclusion: 'success', completed_at: new Date(start).toISOString() },
        { name: 'Rare', conclusion: id % rareEvery === 0 ? 'success' : 'skipped', completed_at: new Date(start).toISOString() },
      ],
    };
    const since = new Date(start - sinceRuns * 60_000).toISOString();
    const runs = await recentPullRequestRuns({ github, owner: 'o', repo: 'r', since });
    return [listWorkflowRuns.mock.callCount(), runs.length];
  };
  assert.deepEqual(await history(1), [1, 50]);
  assert.deepEqual(await history(7), [2, 100]);
  assert.deepEqual(await history(50), [4, 200]);
  assert.deepEqual(await history(50, 70), [2, 71]);
});
