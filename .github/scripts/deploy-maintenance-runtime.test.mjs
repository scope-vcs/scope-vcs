import test from 'node:test';
import assert from 'node:assert/strict';
import { deployMaintenanceRuntime, maintenanceTarget, railwayClient, validateReceipt } from './deploy-maintenance-runtime.mjs';

const id = number => `${String(number).padStart(8, '0')}-1111-4111-8111-111111111111`;
const manifest = { railway: { projectId: id(1), maintenanceServiceId: id(2), databaseServiceId: id(3) }, environments: { staging: { environmentId: id(4) }, production: { environmentId: id(5) } }, services: { api: { id: id(6) } } };
const receipt = { schemaVersion: 1, image: `ghcr.io/scope-vcs/scope-vcs/railway-private-maintenance@sha256:${'a'.repeat(64)}`, apiImage: `ghcr.io/scope-vcs/scope-vcs/railway-private-api@sha256:${'b'.repeat(64)}`, maintenanceSha256: 'c'.repeat(64), apiSourceSha: 'd'.repeat(40), runtimeSourceSha: 'e'.repeat(40) };
const target = maintenanceTarget(manifest, 'staging');
const input = { manifest, environment: 'staging', receipt, credentials: { username: 'durable-user', password: 'durable-secret' } };
const ready = () => ({ id: id(7), serviceId: target.serviceId, environmentId: target.environmentId, status: 'SUCCESS', meta: { imageDigest: receipt.image.split('@')[1], serviceManifest: { source: { image: receipt.image }, deploy: { startCommand: '/app/bin/scope-maintenance serve', healthcheckPath: '/readyz' } } } });

function harness({ deployment = ready(), intercept = () => undefined } = {}) {
  const calls = [], reports = [];
  let time = 0;
  return { calls, reports, options: {
    report: message => reports.push(message), now: () => time, pause: async () => { time += 10_000; }, timeoutMs: 20_000,
    railway: (query, variables) => {
      calls.push({ query, variables });
      const intercepted = intercept(query, variables);
      if (intercepted !== undefined) return intercepted;
      if (query.includes('MaintenanceRuntimeTarget')) return { data: { environment: { projectId: target.projectId, config: { services: { [target.serviceId]: {} } } } } };
      if (query.includes('MaintenanceRuntimeConfig')) return { data: { serviceInstanceUpdate: true } };
      if (query.includes('mutation MaintenanceRuntimeDeploy')) return { data: { serviceInstanceDeployV2: id(7) } };
      return { data: { deployment } };
    },
  } };
}

test('only the dedicated pinned runtime receipt is accepted', () => {
  assert.deepEqual(validateReceipt(receipt), receipt);
  for (const change of [{ schemaVersion: 2 }, { image: receipt.image.replace('@sha256:', ':') }, { image: receipt.apiImage }, { apiImage: receipt.image }, { maintenanceSha256: 'bad' }, { runtimeSourceSha: 'main' }, { apiSourceSha: 'main' }, { serviceId: id(6) }]) {
    assert.throws(() => validateReceipt({ ...receipt, ...change }), /receipt requires/);
  }
});

test('target rejects application/database services, unknown environments and conflicting overrides', () => {
  for (const serviceId of [id(3), id(6), 'scope-maintenance']) assert.throws(() => maintenanceTarget({ ...manifest, railway: { ...manifest.railway, maintenanceServiceId: serviceId } }, 'staging'), /dedicated service/);
  assert.throws(() => maintenanceTarget(manifest, 'preview'), /staging or production/);
  assert.throws(() => maintenanceTarget({ ...manifest, environments: { ...manifest.environments, staging: manifest.environments.production } }, 'staging'), /distinct/);
  for (const key of ['RAILWAY_PROJECT_ID', 'RAILWAY_ENVIRONMENT_ID', 'SCOPE_RAILWAY_ENVIRONMENT_ID', 'RAILWAY_SERVICE_ID', 'SCOPE_RAILWAY_MAINTENANCE_SERVICE_ID']) assert.throws(() => maintenanceTarget(manifest, 'staging', { [key]: id(9) }), /conflicts/);
  assert.equal(maintenanceTarget(manifest, 'production').environmentId, id(5));
});

test('deploy config clears repository settings and deploys only the dedicated service without domains', async () => {
  const h = harness();
  const result = await deployMaintenanceRuntime(input, h.options);
  assert.equal(result.deploymentId, id(7));
  const mutations = h.calls.filter(call => call.query.startsWith('mutation '));
  assert.equal(mutations.length, 2);
  for (const mutation of mutations) {
    assert.equal(mutation.variables.serviceId, target.serviceId);
    assert.equal(mutation.variables.environmentId, target.environmentId);
    assert.doesNotMatch(mutation.query, /domain/i);
  }
  assert.deepEqual(mutations[0].variables.input, { source: { image: receipt.image, repo: null }, rootDirectory: '/', railwayConfigFile: null, buildCommand: null, startCommand: '/app/bin/scope-maintenance serve', healthcheckPath: '/readyz', healthcheckTimeout: 60, preDeployCommand: [], registryCredentials: input.credentials });
  assert.deepEqual(h.reports, [`Maintenance runtime deployment ID: ${id(7)}`]);
});

test('wrong project or absent service fails before mutations', async () => {
  for (const environment of [{ projectId: id(9), config: { services: { [target.serviceId]: {} } } }, { projectId: target.projectId, config: { services: {} } }]) {
    const h = harness({ intercept: query => query.includes('MaintenanceRuntimeTarget') ? { data: { environment } } : undefined });
    await assert.rejects(deployMaintenanceRuntime(input, h.options), /absent/);
    assert.equal(h.calls.length, 1);
  }
});

test('configuration rejection cannot start a deployment', async () => {
  const h = harness({ intercept: query => query.includes('MaintenanceRuntimeConfig') ? { data: { serviceInstanceUpdate: false } } : undefined });
  await assert.rejects(deployMaintenanceRuntime(input, h.options), /did not confirm/);
  assert.equal(h.reports.length, 0);
  assert.equal(h.calls.length, 2);
});

test('uncertain deployment mutation is called once', async () => {
  const h = harness({ intercept: query => { if (query.includes('mutation MaintenanceRuntimeDeploy')) throw new Error('timeout'); } });
  await assert.rejects(deployMaintenanceRuntime(input, h.options), /timeout/);
  assert.equal(h.calls.filter(call => call.query.includes('mutation MaintenanceRuntimeDeploy')).length, 1);
});

test('failure, timeout, mismatched identity and wrong activated configuration fail closed', async () => {
  const cases = [
    { ...ready(), status: 'FAILED' }, { ...ready(), status: 'CRASHED' }, { ...ready(), deploymentStopped: true },
    { ...ready(), serviceId: id(6) }, { ...ready(), environmentId: id(5) }, { ...ready(), id: id(9) },
    { ...ready(), status: 'DEPLOYING' }, null,
  ];
  for (const section of ['image', 'startCommand', 'healthcheckPath', 'imageDigest']) {
    const deployment = ready();
    if (section === 'image') deployment.meta.serviceManifest.source.image = receipt.apiImage;
    else if (section === 'imageDigest') deployment.meta.imageDigest = `sha256:${'f'.repeat(64)}`;
    else deployment.meta.serviceManifest.deploy[section] = 'wrong';
    cases.push(deployment);
  }
  for (const deployment of cases) {
    const h = harness({ deployment });
    await assert.rejects(deployMaintenanceRuntime(input, h.options), /failed or stopped|another target|timed out|does not match/);
    assert.equal(h.calls.filter(call => call.query.includes('mutation MaintenanceRuntimeDeploy')).length, 1);
  }
});

test('Railway mutation secrets stay on stdin, account token is isolated, timeout does not retry or expose errors', () => {
  let count = 0;
  const client = railwayClient({ RAILWAY_API_TOKEN: 'account', RAILWAY_TOKEN: 'project' }, (_command, args, options) => {
    count++;
    assert.equal(options.env.RAILWAY_TOKEN, undefined);
    assert.equal(options.env.RAILWAY_API_TOKEN, 'account');
    assert.equal(options.timeout, 120_000);
    assert.equal(options.killSignal, 'SIGKILL');
    assert.ok(!args.some(arg => arg.includes('durable-secret')));
    assert.ok(options.input.includes('durable-secret'));
    throw new Error('durable-secret');
  });
  assert.throws(() => client('mutation Test{test}', input.credentials), error => /outcome may be unknown/.test(error.message) && !error.message.includes('durable-secret'));
  assert.equal(count, 1);
});
