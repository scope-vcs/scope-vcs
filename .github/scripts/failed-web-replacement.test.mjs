import assert from 'node:assert/strict';
import test from 'node:test';
import { verifyFailedWebReplacement } from './failed-web-replacement.mjs';

const runId = '36389989405';
const sha = 'a'.repeat(40);
const image = `ghcr.io/scope-vcs/scope-web@sha256:${'b'.repeat(64)}`;
const deploymentId = '3ad91699-ffc9-41a6-b560-a9ef31287433';
const serviceId = 'web-service';
const repository = 'scope-vcs/scope-vcs';

function fixture() {
  const run = { id: Number(runId), path: '.github/workflows/release.yml',
    head_branch: 'main', head_sha: sha, event: 'workflow_dispatch',
    status: 'completed', conclusion: 'failure', run_attempt: 1,
    repository: { id: 17, full_name: repository }, head_repository: { id: 17, full_name: repository } };
  const jobs = ['Validate selected components / Production validation gate',
    'Deploy staging / Deploy and smoke staging', 'Prepare Railway artifacts / prepare',
    'Web deploy / Deploy scope-web'].map((name, index) => ({
    name, conclusion: index === 3 ? 'failure' : 'success',
      head_sha: sha, run_id: Number(runId), run_attempt: 1, status: 'completed',
    }));
  const prepared = { schemaVersion: 1, sourceSha: sha, preparationRunId: runId,
    components: { web: { sourceSha: sha, serviceId, image } } };
  const transition = {
    config: { mode: 'ordinary', release: { attemptId: `${runId}:1`, sourceSha: sha, stage: 'web' } },
    deployments: { web: deploymentId },
    summary: { passed: false, release: { attemptId: `${runId}:1`, sourceSha: sha, stage: 'web' },
      failures: [{ target: 'public-homepage', error: { kind: 'application' },
        deployments: { web: deploymentId } }] },
  };
  const deployment = { id: deploymentId, serviceId, status: 'SUCCESS',
    meta: { image, imageDigest: image.split('@')[1] } };
  const manifest = { services: { web: { id: serviceId } },
    environments: { production: { environmentId: 'production' } } };
  const status = { environments: { edges: [{ node: { id: 'production',
    serviceInstances: { edges: [{ node: { serviceId,
      activeDeployments: [{ id: deploymentId, status: 'SUCCESS', deploymentStopped: false }] } }] } } }] } };
  return { runId, repository, run, jobs, prepared, transition, deployment, status, manifest };
}

test('accepts only the exact failed main web activation with immutable image proof', () => {
  const proof = verifyFailedWebReplacement(fixture());
  assert.deepEqual(proof, { sourceSha: sha, provider: 'railway', evidenceId: deploymentId });
});

test('accepts scoped Railway CLI metadata without an optional service ID', () => {
  const value = fixture();
  delete value.deployment.serviceId;
  assert.equal(verifyFailedWebReplacement(value).evidenceId, deploymentId);
});

for (const [name, change] of [
  ['untrusted run', value => { value.run.head_branch = 'feature'; }],
  ['successful run', value => { value.run.conclusion = 'success'; }],
  ['missing staging gate', value => { value.jobs[1].conclusion = 'failure'; }],
  ['later failed staging attempt', value => { value.jobs.push({ ...value.jobs[1], run_attempt: 2, conclusion: 'failure' }); }],
  ['successful web transition', value => { value.jobs[3].conclusion = 'success'; }],
  ['different attempt', value => { value.transition.config.release.attemptId = `${runId}:2`; }],
  ['unrelated probe failure', value => { value.transition.summary.failures[0].target = 'api-readiness'; }],
  ['different activation', value => { value.transition.deployments.web = 'other'; }],
  ['different service', value => { value.deployment.serviceId = 'other-service'; }],
  ['image substitution', value => { value.deployment.meta.imageDigest = `sha256:${'c'.repeat(64)}`; }],
  ['inactive web', value => { value.status.environments.edges[0].node.serviceInstances.edges[0].node.activeDeployments = []; }],
]) {
  test(`rejects ${name}`, () => {
    const value = fixture();
    change(value);
    assert.throws(() => verifyFailedWebReplacement(value));
  });
}
