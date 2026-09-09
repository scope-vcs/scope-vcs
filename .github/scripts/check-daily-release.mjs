import { execFileSync } from 'node:child_process';
import { appendFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

const chicago = new Intl.DateTimeFormat('en-CA', {
  timeZone: 'America/Chicago', year: 'numeric', month: '2-digit', day: '2-digit',
  hour: '2-digit', hourCycle: 'h23',
});

function localTime(value) {
  const parts = Object.fromEntries(chicago.formatToParts(new Date(value)).map(({ type, value }) => [type, value]));
  return { date: `${parts.year}-${parts.month}-${parts.day}`, hour: Number(parts.hour) };
}

// Like T3's nightly check, this runs inside the release concurrency lock.
// Scope's successful production health job is its release receipt.
export async function isDailyReleaseDue({ event, now = new Date(), request }) {
  if (event !== 'schedule') return true;
  const today = localTime(now);
  if (today.hour < 9) return false;
  // Include overnight releases; individual jobs supply the completion date.
  const since = new Date(new Date(now).getTime() - 48 * 60 * 60 * 1000).toISOString();
  for (let page = 1; ; page++) {
    const { workflow_runs: runs } = await request(`/actions/workflows/scope-production-deploy.yml/runs?branch=main&status=success&created=${encodeURIComponent(`>=${since}`)}&per_page=100&page=${page}`);
    if (!Array.isArray(runs)) throw new Error('Cannot read successful release runs');
    for (const run of runs) {
      if (!['schedule', 'workflow_dispatch'].includes(run.event) || run.head_branch !== 'main'
          || run.status !== 'completed' || run.conclusion !== 'success') continue;
      for (let jobPage = 1; ; jobPage++) {
        const { jobs } = await request(`/actions/runs/${run.id}/jobs?filter=latest&per_page=100&page=${jobPage}`);
        if (!Array.isArray(jobs)) throw new Error('Cannot read production health jobs');
        if (jobs.some((job) => job.name === 'Production Railway health gate'
            && job.run_id === run.id && job.head_sha === run.head_sha
            && job.status === 'completed' && job.conclusion === 'success'
            && job.completed_at && localTime(job.completed_at).date === today.date)) return false;
        if (jobs.length < 100) break;
      }
    }
    if (runs.length < 100) return true;
  }
}

async function main() {
  const repository = process.env.GITHUB_REPOSITORY;
  if (!repository || !process.env.GITHUB_OUTPUT) throw new Error('GitHub repository and output are required');
  const due = await isDailyReleaseDue({
    event: process.env.GITHUB_EVENT_NAME,
    request: async (path) => JSON.parse(execFileSync('gh', ['api', `/repos/${repository}${path}`], { encoding: 'utf8', timeout: 30_000 })),
  });
  appendFileSync(process.env.GITHUB_OUTPUT, `due=${due}\n`);
  console.log(due ? 'Release is eligible; the component planner will skip unchanged components.' : 'Automatic release is not due yet or already succeeded today.');
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => { console.error(error.message); process.exitCode = 1; });
}
