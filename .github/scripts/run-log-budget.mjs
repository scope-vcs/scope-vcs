import { posthogRequest } from '../../dev/analytics/sync-reports.mjs';

export const ISSUE_TITLE = 'Run page logs are over their time budget';
const MIN_SAMPLES = 20;
const P75_BUDGET_MS = { job_click: 400, list_click: 800, page_load: 2000 };
const MIN_STORED_PERCENT = 95;
const SECRETS = ['POSTHOG_PERSONAL_API_KEY', 'POSTHOG_PROJECT_ID'];

export const BUDGET_QUERY = `SELECT properties.trigger AS trigger, count() AS samples,
  quantile(0.75)(toFloat(properties.duration_ms)) AS p75_ms,
  countIf(properties.source = 'stored') AS stored
FROM events
WHERE event = 'run_log_timing' AND properties.environment = 'production'
  AND timestamp >= now() - INTERVAL 7 DAY
GROUP BY trigger`;

export function judgeRunLogBudgets(rows) {
  const judged = [];
  for (const [trigger, budgetMs] of Object.entries(P75_BUDGET_MS)) {
    const row = rows.find(item => item.trigger === trigger);
    if (!row || row.samples < MIN_SAMPLES) continue;
    judged.push({
      budget: `p75 \`${trigger}\` at most ${budgetMs} ms`,
      value: `${Math.round(row.p75Ms)} ms`,
      samples: row.samples,
      breached: row.p75Ms > budgetMs,
    });
  }
  const samples = rows.reduce((total, row) => total + row.samples, 0);
  const stored = rows.reduce((total, row) => total + row.stored, 0);
  if (samples >= MIN_SAMPLES) {
    const percent = (100 * stored) / samples;
    judged.push({
      budget: `at least ${MIN_STORED_PERCENT}% served from stored logs`,
      value: `${percent.toFixed(1)}%`,
      samples,
      breached: percent < MIN_STORED_PERCENT,
    });
  }
  return { judged: judged.length, breaches: judged.filter(item => item.breached) };
}

function breachIssueBody(breaches, runUrl) {
  return [
    `Over the last 7 days of production \`run_log_timing\` events, the run page broke these log budgets. Budgets with fewer than ${MIN_SAMPLES} samples are not judged.`,
    '',
    '| Budget | Value | Samples |',
    '| --- | --- | --- |',
    ...breaches.map(breach => `| ${breach.budget} | ${breach.value} | ${breach.samples} |`),
    '',
    `[Budget check](${runUrl})`,
  ].join('\n');
}

export async function readRunLogTimings(request) {
  const { columns, results } = await request('query/', {
    method: 'POST',
    body: { query: { kind: 'HogQLQuery', query: BUDGET_QUERY }, name: 'Run log budget', refresh: 'blocking' },
  });
  return results.map(values => {
    const row = Object.fromEntries(columns.map((column, index) => [column, values[index]]));
    return { trigger: row.trigger, samples: Number(row.samples), p75Ms: Number(row.p75_ms), stored: Number(row.stored) };
  });
}

export async function reportRunLogBudget({ github, owner, repo, runUrl, env, request }) {
  for (const name of SECRETS) {
    if (!env[name]) throw new Error(`${name} is not set`);
  }
  const query = request ?? posthogRequest({ apiKey: env.POSTHOG_PERSONAL_API_KEY, projectId: env.POSTHOG_PROJECT_ID });
  const { judged, breaches } = judgeRunLogBudgets(await readRunLogTimings(query));
  const issues = await github.paginate(github.rest.issues.listForRepo, { owner, repo, state: 'open', per_page: 100 });
  const existing = issues.find(issue => !issue.pull_request && issue.title === ISSUE_TITLE);
  if (breaches.length) {
    const body = breachIssueBody(breaches, runUrl);
    if (existing) return github.rest.issues.update({ owner, repo, issue_number: existing.number, body });
    return github.rest.issues.create({ owner, repo, title: ISSUE_TITLE, body, labels: ['bug'] });
  }
  if (!existing || judged === 0) return null;
  await github.rest.issues.createComment({
    owner, repo, issue_number: existing.number, body: `Every judged run log budget passes. [Budget check](${runUrl})`,
  });
  return github.rest.issues.update({ owner, repo, issue_number: existing.number, state: 'closed', state_reason: 'completed' });
}
