import { execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { assertActivatedArtifact, validatePreparedRelease } from './railway-artifact.mjs';
import { githubRequest } from './production-deployment-progress.mjs';
import { validateRecoveryPreparation } from './recovery-preparation-trust.mjs';
import { readRailway } from './railway-read.mjs';

const shaPattern = /^[a-f0-9]{40}$/;
const runPattern = /^[1-9][0-9]*$/;
const releasePath = '.github/workflows/release.yml';

function requireMatch(condition, message) {
  if (!condition) throw new Error(`Failed web replacement: ${message}`);
}

export function verifyFailedWebReplacement({ runId, repository, run, jobs, prepared, transition, deployment, status, manifest }) {
  requireMatch(runPattern.test(runId), 'run ID must be numeric');
  requireMatch(run.id === Number(runId) && run.path === releasePath && run.head_branch === 'main'
    && run.event === 'workflow_dispatch' && run.status === 'completed' && run.conclusion === 'failure'
    && run.repository?.full_name === repository && run.head_repository?.full_name === repository
    && run.repository?.id === run.head_repository?.id && shaPattern.test(run.head_sha),
  'source must be a failed Release run from this repository on main');
  requireMatch(Array.isArray(jobs), 'cannot inspect source jobs');
  const latest = name => jobs.filter(job => job.name === name && job.head_sha === run.head_sha
    && String(job.run_id) === runId && job.status === 'completed')
    .sort((left, right) => right.run_attempt - left.run_attempt)[0];
  requireMatch(['Validate selected components / Production validation gate',
    'Deploy staging / Deploy and smoke staging'].every(name => latest(name)?.conclusion === 'success')
    && jobs.some(job => job.name === 'Prepare Railway artifacts / prepare'
      && job.conclusion === 'success' && job.status === 'completed'
      && job.head_sha === run.head_sha && String(job.run_id) === runId),
  'validation, staging, and preparation must have succeeded');
  requireMatch(jobs.some(job => job.name === 'Web deploy / Deploy scope-web'
    && job.conclusion === 'failure' && job.head_sha === run.head_sha
    && String(job.run_id) === runId && job.run_attempt === run.run_attempt),
  'latest web transition must have failed');
  validatePreparedRelease(prepared, { sourceSha: run.head_sha, components: ['web'], services: manifest.services });
  requireMatch(prepared.preparationRunId === runId, 'prepared image belongs to another run');
  const { config, deployments, summary } = transition;
  requireMatch(config?.release?.attemptId === `${runId}:${run.run_attempt}`
    && config.release.sourceSha === run.head_sha && config.release.stage === 'web'
    && config.mode === 'ordinary', 'transition artifact does not match the failed attempt');
  requireMatch(summary?.passed === false && summary?.release?.attemptId === config.release.attemptId
    && summary.release.sourceSha === run.head_sha && summary.release.stage === 'web'
    && summary.failures?.length > 0 && summary.failures.every(failure =>
      failure.target === 'public-homepage' && failure.error?.kind === 'application'),
  'retained probe did not fail solely on the homepage contract');
  const deploymentId = deployments?.web;
  requireMatch(typeof deploymentId === 'string' && deploymentId.length > 0
    && summary.failures.some(failure => failure.deployments?.web === deploymentId),
  'transition did not observe the failed web deployment');
  assertActivatedArtifact(prepared, 'web', deployment, { deploymentId });
  const environment = status?.environments?.edges?.map(({ node }) => node)
    .find(node => node.id === manifest.environments.production.environmentId);
  const instance = environment?.serviceInstances?.edges?.map(({ node }) => node)
    .find(node => node.serviceId === prepared.components.web.serviceId);
  requireMatch(instance?.activeDeployments?.some(active => active.id === deploymentId
    && active.status === 'SUCCESS' && active.deploymentStopped !== true),
  'retained deployment is not actively serving production web');
  return { sourceSha: run.head_sha, provider: 'railway', evidenceId: deploymentId };
}

export async function loadFailedWebReplacement(runId, { repository, status, manifest,
  request = githubRequest, execute = execFileSync, railway = readRailway } = {}) {
  requireMatch(runPattern.test(runId), 'run ID must be numeric');
  const run = await request(`/actions/runs/${runId}`);
  requireMatch(shaPattern.test(run.head_sha), 'run has no exact source SHA');
  const jobs = [];
  for (let page = 1; ; page += 1) {
    const result = await request(`/actions/runs/${runId}/jobs?filter=all&per_page=100&page=${page}`);
    requireMatch(Array.isArray(result.jobs), 'cannot inspect run jobs');
    jobs.push(...result.jobs);
    if (result.jobs.length < 100) break;
  }
  const directory = mkdtempSync(join(tmpdir(), 'scope-failed-web-'));
  try {
    for (const artifact of [`prepared-release-${run.head_sha}`,
      `production-web-transition-${run.head_sha}-${run.run_attempt}`]) {
      execute('gh', ['run', 'download', runId, '--repo', repository, '--name', artifact,
        '--dir', join(directory, artifact)], { stdio: 'pipe' });
    }
    const prepared = JSON.parse(readFileSync(join(directory, `prepared-release-${run.head_sha}`, 'prepared-release.json')));
    await validateRecoveryPreparation(prepared, request, repository, manifest, ['web']);
    const evidence = join(directory, `production-web-transition-${run.head_sha}-${run.run_attempt}`, 'web');
    const transition = {
      config: JSON.parse(readFileSync(join(evidence, 'config.json'))),
      deployments: JSON.parse(readFileSync(join(evidence, 'deployments.json'))),
      summary: JSON.parse(readFileSync(join(evidence, 'availability-summary.json'))),
    };
    const serviceId = prepared.components?.web?.serviceId;
    requireMatch(serviceId === manifest.services.web.id, 'prepared web service ID differs from production');
    const deployments = railway(['deployment', 'list', '--project', manifest.railway.projectId,
      '--service', serviceId, '--environment', manifest.environments.production.environmentId,
      '--limit', '100', '--json']);
    requireMatch(Array.isArray(deployments), 'Railway deployment list is invalid');
    const deployment = deployments.find(item => item.id === transition.deployments?.web);
    return verifyFailedWebReplacement({ runId, repository, run, jobs, prepared, transition,
      deployment, status, manifest });
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const runId = process.env.SCOPE_REPLACE_FAILED_WEB_RUN_ID;
    requireMatch(process.env.SCOPE_REQUESTED_SCOPE === 'web', 'replacement requires web scope');
    requireMatch(runPattern.test(runId ?? ''), 'run ID must be numeric');
    const replacement = await loadFailedWebReplacement(runId, {
      repository: process.env.GITHUB_REPOSITORY,
      status: JSON.parse(process.env.SCOPE_RAILWAY_SERVICES_JSON),
      manifest: JSON.parse(process.env.SCOPE_DEPLOYMENT_MANIFEST_JSON),
    });
    process.stdout.write(`${JSON.stringify(replacement)}\n`);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
