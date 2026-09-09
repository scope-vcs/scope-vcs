import test from 'node:test';
import assert from 'node:assert/strict';
import { snapshotGate, enterGate, restoreGateConfiguration, verifyGateDeployment } from './railway-maintenance-gate.mjs';
const base = { projectId: '11111111-1111-1111-1111-111111111111', environmentId: '22222222-2222-2222-2222-222222222222', serviceId: '33333333-3333-3333-3333-333333333333', image: `ghcr.io/scope-vcs/scope-api@sha256:${'a'.repeat(64)}` };
const previous = { source: { image: 'previous-image' }, build: { buildCommand: null, rootDirectory: '/', railwayConfigFile: null }, deploy: { startCommand: 'normal-server', healthcheckPath: '/healthz', healthcheckTimeout: 30, preDeployCommand: null } };
test('snapshot persists only changed configuration before gate mutations', () => {
  let saved;
  const gate = snapshotGate(base, { persist: value => { saved = value; }, railway: () => ({ data: { environment: { projectId: base.projectId, config: { services: { [base.serviceId]: { ...previous, source: { ...previous.source, rootDirectory: '/' }, variables: { SECRET: 'hidden' } } } } } } }) });
  assert.deepEqual(gate.previous, { ...previous, source: { ...previous.source, rootDirectory: '/' } });
  assert.equal(saved.phase, 'snapshotted');
  assert.equal(JSON.stringify(saved).includes('hidden'), false);
});
test('activation records uncertainty before deploy and exact ID afterward', () => {
  const events = [];
  const gate = enterGate({ ...base, previous, phase: 'snapshotted' }, { persist: v => events.push(v), railway: (query, variables) => {
    if (query.includes('MaintenanceGateConfig')) {
      assert.equal(variables.input.startCommand, '/app/bin/scope-maintenance serve');
      assert.equal(variables.input.healthcheckPath, '/readyz');
      assert.deepEqual(variables.input.preDeployCommand, []);
      return { data: { serviceInstanceUpdate: true } };
    }
    assert.equal(events.at(-1).phase, 'deploying');
    return { data: { serviceInstanceDeployV2: 'exact-id' } };
  } });
  assert.equal(gate.deploymentId, 'exact-id');
  assert.equal(events.at(-1).deploymentId, 'exact-id');
});
test('unknown deployment outcome cannot silently launch a second gate', () => {
  assert.throws(() => enterGate({ ...base, previous, phase: 'deploying' }, { railway: () => assert.fail(), persist: () => assert.fail() }), /reconcile/);
});
test('restoration removes temporary command and healthcheck without launching old code', () => {
  const calls = [];
  restoreGateConfiguration({ ...base, previous }, { persist: v => assert.equal(v.phase, 'restored'), railway: (query, variables) => { calls.push(query); assert.equal(variables.input.startCommand, 'normal-server'); assert.equal(variables.input.healthcheckPath, '/healthz'); assert.deepEqual(variables.input.source, previous.source); return { data: { serviceInstanceUpdate: true } }; } });
  assert.equal(calls.length, 1);
});
test('readiness requires exact gate identity, image and effective maintenance config', () => {
  const gate = { ...base, deploymentId: 'exact-id' };
  const deployment = { id: 'exact-id', serviceId: base.serviceId, status: 'SUCCESS', meta: { serviceManifest: { source: { image: base.image }, deploy: { startCommand: '/app/bin/scope-maintenance serve', healthcheckPath: '/readyz' } } } };
  verifyGateDeployment(gate, deployment);
  assert.throws(() => verifyGateDeployment(gate, { ...deployment, id: 'other' }), /exact/);
  deployment.meta.serviceManifest.deploy.startCommand = 'normal-server';
  assert.throws(() => verifyGateDeployment(gate, deployment), /configuration/);
});
test('snapshot refuses a service from another project', () => {
  assert.throws(() => snapshotGate(base, { persist: () => assert.fail(), railway: () => ({ data: { environment: { projectId: 'other' } } }) }), /project/);
});
test('interrupted activation reconciles a unique exact gate without redeploying', () => {
  const gate = { ...base, previous, phase: 'deploying', capturedAt: '2026-09-09T12:00:00Z' };
  const candidate = { id: 'reconciled', serviceId: base.serviceId, createdAt: '2026-09-09T12:01:00Z', status: 'SUCCESS', meta: { serviceManifest: { source: { image: base.image }, deploy: { startCommand: '/app/bin/scope-maintenance serve' } } } };
  const options = { railway: () => assert.fail('must not deploy'), persist: () => {}, deployments: () => [candidate] };
  assert.equal(enterGate(gate, options).deploymentId, 'reconciled');
  assert.throws(() => enterGate(gate, { ...options, deployments: () => [candidate, { ...candidate, id: 'duplicate' }] }), /Multiple/);
  assert.throws(() => enterGate(gate, { ...options, deployments: () => [{ ...candidate, createdAt: '2026-09-08T12:00:00Z' }] }), /reconcile/);
});
test('reclose retains original configuration and replaces removed gate identity', async () => {
  const { recloseGate } = await import('./railway-maintenance-gate.mjs');
  const gate = { ...base, previous, deploymentId: 'removed-gate', phase: 'active' };
  const reset = recloseGate(gate, { deployments: () => [{ id: 'new-writer', serviceId: base.serviceId, status: 'SUCCESS' }, { id: 'removed-gate', status: 'REMOVED' }], persist: () => {}, railway: query => ({ data: query.includes('MaintenanceGateConfig') ? { serviceInstanceUpdate: true } : { serviceInstanceDeployV2: 'new-gate' } }) });
  assert.equal(reset.deploymentId, 'new-gate');
  assert.deepEqual(reset.previous, previous);
  assert.deepEqual(reset.predecessorIds, ['new-writer']);
});
test('predecessor shutdown targets exact old deployment and refuses gate identity', async () => {
  const { stopGatePredecessors } = await import('./railway-maintenance-gate.mjs');
  const stopped = [];
  const railway = (query, { id }) => {
    if (query.startsWith('query')) return { data: { deployment: { id, serviceId: base.serviceId, status: 'SUCCESS' } } };
    stopped.push(id);
    return { data: { deploymentStop: true } };
  };
  stopGatePredecessors({ ...base, deploymentId: 'gate', predecessorIds: ['old'] }, { railway });
  assert.deepEqual(stopped, ['old']);
  assert.throws(() => stopGatePredecessors({ ...base, deploymentId: 'gate', predecessorIds: ['gate'] }, { railway }), /Refusing/);
});
