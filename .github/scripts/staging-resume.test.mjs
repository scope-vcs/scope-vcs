import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import { validateStagingResumeEvidence, validateStagingResumeDeployments, validateStagingResumeSchema } from './staging-resume.mjs';

const manifest = JSON.parse(readFileSync(new URL('../deployment-services.json', import.meta.url)));

function fixture() {
  const sourceSha = 'a'.repeat(40);
  const prepared = { schemaVersion: 1, sourceSha, components: Object.fromEntries(
    ['api', 'run-worker', 'cache', 'git-router', 'media-api', 'media-worker', 'web'].map(component => [component, {
      sourceSha, serviceId: manifest.services[component].id,
      image: `ghcr.io/scope-vcs/${component}@sha256:${'b'.repeat(64)}`,
    }]),
  ) };
  const environmentId = manifest.environments.staging.environmentId;
  const evidence = { commit: sourceSha, environmentId, candidateDeployments: 1, deployments:
    Object.keys(prepared.components).map(component => ({
      service: component === 'git-router' ? manifest.environments.staging.routerServiceId : manifest.services[component].id,
      deploymentId: `deployment-${component}`, status: 'SUCCESS',
    })),
  };
  const histories = Object.fromEntries(evidence.deployments.map((row, index) => [row.service, [{
    id: row.deploymentId, status: 'REMOVED',
    meta: { image: Object.values(prepared.components)[index].image },
  }]]));
  return { prepared, evidence, histories };
}

test('staging resume accepts exact recorded images removed by failed-smoke cleanup', () => {
  const { prepared, evidence, histories } = fixture();
  validateStagingResumeDeployments(prepared, evidence, manifest, histories);
  validateStagingResumeSchema({ exact: true, pending: [] });
});

test('staging resume rejects wrong, missing, duplicate and incomplete deployment evidence', () => {
  for (const mutate of [
    s => { s.evidence.commit = 'c'.repeat(40); },
    s => { s.evidence.environmentId = manifest.environments.production.environmentId; },
    s => { s.evidence.candidateDeployments = 0; },
    s => { s.evidence.deployments.pop(); },
    s => { s.evidence.deployments[0] = s.evidence.deployments[1]; },
    s => { s.evidence.deployments[0].status = 'FAILED'; },
    s => { s.evidence.deployments[0].deploymentId = ''; },
    s => { s.evidence.deployments[0].deploymentId = s.evidence.deployments[1].deploymentId; },
  ]) {
    const state = fixture();
    mutate(state);
    assert.throws(() => validateStagingResumeEvidence(state.prepared, state.evidence, manifest));
  }
});

test('staging resume rejects changed images, intervening deployments, and unproven provider state', () => {
  for (const mutate of [
    history => { history[0].meta.image = history[0].meta.image.replace('b'.repeat(64), 'c'.repeat(64)); },
    history => { history[0].meta = {}; },
    history => { history[0].status = 'DEPLOYING'; },
    history => { history[0].id = 'unrecorded'; },
    history => { history[0].serviceId = 'wrong-service'; },
    history => { history[0].environmentId = manifest.environments.production.environmentId; },
    history => { history.unshift({ ...history[0], id: 'newer', meta: { image: 'ghcr.io/other/image@sha256:' + 'd'.repeat(64) } }); },
    history => { history.length = 0; },
  ]) {
    const { prepared, evidence, histories } = fixture();
    mutate(histories[evidence.deployments[0].service]);
    assert.throws(() => validateStagingResumeDeployments(prepared, evidence, manifest, histories));
  }
  for (const plan of [null, {}, { exact: false, pending: [] }, { exact: true, pending: [{ name: 'migration' }] }]) {
    assert.throws(() => validateStagingResumeSchema(plan));
  }
});
