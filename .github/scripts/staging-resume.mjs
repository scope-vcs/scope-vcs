import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { assertDeploymentArtifact, validateMaintenanceArtifact } from './railway-artifact.mjs';
import { validatePreparedDeployment } from './release-selection.mjs';
import { githubRequest } from './production-deployment-progress.mjs';
import { readRailway } from './railway-read.mjs';

// An interrupted smoke run can resume only the images whose complete staging
// deployment was recorded by the original, validated release on main.
export function validateStagingResumeEvidence(prepared, evidence, manifest) {
  const environment = manifest.environments.staging.environmentId;
  assert.notEqual(environment, manifest.environments.production.environmentId);
  assert.equal(evidence?.environmentId, environment, 'Staging evidence targets another environment');
  assert.equal(evidence?.commit, prepared.sourceSha, 'Staging evidence targets another source');
  assert.equal(evidence?.candidateDeployments, 1, 'Staging evidence must record a complete candidate deployment');
  assert(Array.isArray(evidence.deployments), 'Staging deployment evidence is missing');
  const targets = Object.entries(prepared.components).map(([component, artifact]) => ({
    component,
    serviceId: artifact.serviceId,
  }));
  assert.equal(evidence.deployments.length, targets.length, 'Staging evidence has unexpected components');
  for (const { serviceId } of targets) {
    const rows = evidence.deployments.filter(row => row.service === serviceId);
    assert.equal(rows.length, 1, 'Staging evidence must bind each service exactly once');
    assert.equal(rows[0].status, 'SUCCESS', 'Staging deployment did not succeed');
    assert(typeof rows[0].deploymentId === 'string' && rows[0].deploymentId.length > 0,
      'Staging evidence is missing a deployment ID');
  }
  assert.equal(new Set(evidence.deployments.map(row => row.deploymentId)).size, targets.length,
    'Staging deployment IDs must be distinct');
  return targets;
}

export function validateStagingResumeDeployments(prepared, evidence, manifest, histories) {
  for (const { component, serviceId } of validateStagingResumeEvidence(prepared, evidence, manifest)) {
    const history = histories[serviceId];
    assert(Array.isArray(history) && history.length > 0, `No staging history for ${component}`);
    const recorded = evidence.deployments.find(row => row.service === serviceId);
    const original = history.find(deployment => deployment.id === recorded.deploymentId);
    assert(original, `Original staging deployment is unavailable for ${component}`);
    const scoped = { ...prepared, components: {
      ...prepared.components, [component]: { ...prepared.components[component], serviceId },
    } };
    // Cleanup removes writers after smoke failure. REMOVED preserves immutable
    // image evidence; the original GitHub step and receipt establish success.
    for (const deployment of [original, history[0]]) {
      assert(['SUCCESS', 'REMOVED'].includes(deployment.status),
        `Staging ${component} has an unfinished or unsuccessful deployment`);
      // The CLI omits these IDs from scoped lists; reject contradictions when
      // present, and always obtain each history with explicit service/env IDs.
      if (deployment.serviceId) assert.equal(deployment.serviceId, serviceId,
        'Staging deployment targets another service');
      if (deployment.environmentId) assert.equal(deployment.environmentId, manifest.environments.staging.environmentId,
        'Staging deployment targets another environment');
      assertDeploymentArtifact(scoped, component, deployment, { deploymentId: deployment.id });
    }
  }
}

export function validateStagingResumeSchema(plan) {
  assert(plan?.exact === true && Array.isArray(plan.pending) && plan.pending.length === 0,
    'Staging resume requires the pinned candidate schema with no pending migrations');
}

async function main() {
  const [preparedPath, evidencePath, binaryPath] = process.argv.slice(2);
  const manifest = JSON.parse(readFileSync(process.env.SCOPE_DEPLOYMENT_MANIFEST || '.github/deployment-services.json'));
  const prepared = JSON.parse(readFileSync(preparedPath));
  const evidence = JSON.parse(readFileSync(evidencePath));
  await validatePreparedDeployment(prepared, process.env.SOURCE_RUN_ID, githubRequest,
    process.env.GITHUB_REPOSITORY, { resumeStaging: true });
  validateMaintenanceArtifact(prepared, readFileSync(binaryPath));
  const targets = validateStagingResumeEvidence(prepared, evidence, manifest);
  const scope = ['--project', manifest.railway.projectId, '--environment', manifest.environments.staging.environmentId];
  const histories = Object.fromEntries(targets.map(({ serviceId }) => [serviceId,
    readRailway(['deployment', 'list', ...scope, '--service', serviceId, '--limit', '100', '--json']),
  ]));
  validateStagingResumeDeployments(prepared, evidence, manifest, histories);
  const variables = readRailway(['variable', 'list', ...scope, '--service', manifest.railway.databaseServiceId, '--json']);
  assert(variables.DATABASE_PUBLIC_URL, 'Staging database endpoint is missing');
  const plan = JSON.parse(execFileSync(binaryPath, ['plan'], {
    encoding: 'utf8', timeout: 60_000,
    env: { ...process.env, DATABASE_URL: variables.DATABASE_PUBLIC_URL },
    stdio: ['ignore', 'pipe', 'pipe'],
  }));
  validateStagingResumeSchema(plan);
  console.log(`Verified staging resume for ${prepared.sourceSha} from run ${prepared.preparationRunId}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
