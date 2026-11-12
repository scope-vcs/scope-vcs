import assert from 'node:assert/strict';
import test from 'node:test';
import { productionTargetFromEnvironment, verifyProductionTarget, verifyRouterTopology } from './railway-production-target.mjs';

const region = 'us-east4-eqdc4a';
const target = productionTargetFromEnvironment({
  RAILWAY_PROJECT_ID: 'project', SCOPE_RAILWAY_ENVIRONMENT_ID: 'production-id', SCOPE_RAILWAY_ROUTER_GROUP_ID: 'group',
  SCOPE_RAILWAY_API_SERVICE_ID: 'api', SCOPE_RAILWAY_WORKER_SERVICE_ID: 'worker', SCOPE_RAILWAY_ROUTER_SERVICE_ID: 'router',
  SCOPE_RAILWAY_MEDIA_SERVICE_ID: 'media', SCOPE_RAILWAY_MEDIA_WORKER_SERVICE_ID: 'media-worker',
  SCOPE_RAILWAY_DATABASE_SERVICE_ID: 'postgres', SCOPE_RAILWAY_API_REGION_ID: region,
  SCOPE_RAILWAY_WORKER_REGION_ID: region, SCOPE_RAILWAY_MEDIA_REGION_ID: region,
});
const ids = Object.values(target.services);

function state() {
  return {
    status: {
      id: 'project',
      environments: { edges: [{ node: { id: 'production-id', name: 'production' } }] },
      services: { edges: ids.map(({ id, name }) => ({ node: { id, name } })) },
    },
    services: ids.filter(({ id }) => id !== 'postgres').map(({ id }) => ({
      id, status: 'SUCCESS', replicas: { configured: 1, running: 1, crashed: 0 }, regions: [{ name: region, configured: 1 }],
    })),
    environmentConfig: { services: Object.fromEntries(ids.map(({ id }) => [id, {
      groupId: id === 'router' ? 'group' : undefined,
      deploy: { multiRegionConfig: { [region]: { numReplicas: 1 } } },
    }])) },
  };
}

test('accepts the reviewed production target and router topology', () => {
  verifyProductionTarget(state(), target);
  verifyRouterTopology(state(), target);
});

test('rejects a different project, environment, service, or live or stored region', () => {
  for (const [mutate, message] of [
    [(input) => { input.status.id = 'other'; }, /project ID/],
    [(input) => { input.status.environments.edges[0].node.name = 'staging'; }, /production environment/],
    [(input) => { input.status.services.edges.pop(); }, /scope-postgres does not match/],
    [(input) => { input.services = input.services.filter(({ id }) => id !== 'media'); }, /scope-media-api has no production state/],
    [(input) => { input.services[0].regions[0].name = 'us-west2'; }, /scope-api is not running in reviewed region/],
    [(input) => { input.environmentConfig.services.api.deploy.multiRegionConfig = { 'us-west2': { numReplicas: 1 } }; }, /scope-api stored region config/],
    [(input) => { input.environmentConfig.services.worker.deploy.multiRegionConfig[region].numReplicas = 2; }, /scope-run-worker stored region config/],
  ]) {
    const input = state();
    mutate(input);
    assert.throws(() => verifyProductionTarget(input, target), message);
  }
});

test('stopped regional services skip region checks', () => {
  const input = state();
  Object.assign(input.services.find(({ id }) => id === 'media'), { replicas: { configured: 0 }, regions: [] });
  input.environmentConfig.services.media.deploy.multiRegionConfig = {};
  verifyProductionTarget(input, target);
});

test('router topology requires one healthy replica in the API region and group', () => {
  const router = (input) => input.services.find(({ id }) => id === 'router');
  for (const mutate of [
    (input) => { input.environmentConfig.services.router.groupId = 'other'; },
    (input) => { router(input).status = 'CRASHED'; },
    (input) => { router(input).deploymentStopped = true; },
    (input) => { router(input).replicas = { configured: 2, running: 2, crashed: 0 }; },
    (input) => { router(input).replicas.crashed = 1; },
    (input) => { router(input).regions = [{ name: 'us-west2', configured: 1 }]; },
    (input) => { input.environmentConfig.services.router.deploy.multiRegionConfig['us-west2'] = { numReplicas: 1 }; },
    (input) => { input.services = input.services.filter(({ id }) => id !== 'router'); },
  ]) {
    const input = state();
    mutate(input);
    assert.throws(() => verifyRouterTopology(input, target), /router topology/);
  }
});
