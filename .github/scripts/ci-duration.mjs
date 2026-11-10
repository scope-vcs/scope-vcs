export const BUDGET_SECONDS = 450;
export const SAMPLE_SIZE = 10;
export const ISSUE_TITLE = 'Pull request CI is over its time budget';

export function slowJobs(runs, { budgetSeconds = BUDGET_SECONDS, sampleSize = SAMPLE_SIZE } = {}) {
  const samples = new Map();
  for (const run of runs) {
    const started = Date.parse(run.startedAt);
    for (const job of run.jobs) {
      if (job.conclusion !== 'success') continue;
      const runs = samples.get(job.name) ?? [];
      if (runs.length < sampleSize) runs.push({ seconds: (Date.parse(job.completedAt) - started) / 1000, url: run.url });
      samples.set(job.name, runs);
    }
  }
  return [...samples]
    .filter(([, runs]) => runs.length === sampleSize)
    .map(([name, runs]) => ({ name, medianSeconds: median(runs.map(run => run.seconds)), runs }))
    .filter(job => job.medianSeconds > budgetSeconds)
    .sort((left, right) => right.medianSeconds - left.medianSeconds);
}

function median(values) {
  const sorted = [...values].sort((left, right) => left - right);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
}

export function durationIssueBody(jobs, { runUrl, budgetSeconds = BUDGET_SECONDS, sampleSize = SAMPLE_SIZE }) {
  const minutes = seconds => (seconds / 60).toFixed(1);
  return [
    `Over its last ${sampleSize} successful pull request runs, each job below took longer than ${minutes(budgetSeconds)} minutes, measured from run start to job end. The median is shown.`,
    '',
    '| Job | Median minutes | Slowest sampled run |',
    '| --- | --- | --- |',
    ...jobs.map(job => {
      const slowest = job.runs.reduce((left, right) => (right.seconds > left.seconds ? right : left));
      return `| \`${job.name.replace(/[`\r\n|]/g, '')}\` | ${minutes(job.medianSeconds)} | [${minutes(slowest.seconds)} min](${slowest.url}) |`;
    }),
    '',
    `[Duration check](${runUrl})`,
  ].join('\n');
}

export async function recentPullRequestRuns({ github, owner, repo, count = 50 }) {
  const { data } = await github.rest.actions.listWorkflowRuns({
    owner, repo, workflow_id: 'ci.yml', event: 'pull_request', status: 'success', per_page: count,
  });
  const runs = [];
  const sameRepository = run => run.head_repository?.full_name === `${owner}/${repo}`;
  for (const run of data.workflow_runs.filter(run => run.run_attempt === 1 && sameRepository(run))) {
    const jobs = await github.paginate(github.rest.actions.listJobsForWorkflowRun, {
      owner, repo, run_id: run.id, per_page: 100,
    });
    runs.push({
      url: run.html_url,
      startedAt: run.run_started_at,
      jobs: jobs.map(job => ({ name: job.name, conclusion: job.conclusion, completedAt: job.completed_at })),
    });
  }
  return runs;
}

export async function reportCiDuration({ github, owner, repo, runUrl }) {
  const jobs = slowJobs(await recentPullRequestRuns({ github, owner, repo }));
  if (!jobs.length) return null;
  const body = durationIssueBody(jobs, { runUrl });
  const issues = await github.paginate(github.rest.issues.listForRepo, { owner, repo, state: 'open', per_page: 100 });
  const existing = issues.find(issue => !issue.pull_request && issue.title === ISSUE_TITLE);
  if (existing) return github.rest.issues.createComment({ owner, repo, issue_number: existing.number, body });
  return github.rest.issues.create({ owner, repo, title: ISSUE_TITLE, body, labels: ['bug'] });
}
