import assert from 'node:assert/strict';
import test from 'node:test';
import { loadRedeployedReplacement, verifyRedeployedReplacement } from './redeployed-service-replacement.mjs';

const digest = `sha256:${'a'.repeat(64)}`;
const unreceiptedChange = /not a redeploy, rollback, or variable change of a receipted image/;
const receipt = { sourceSha: 'b'.repeat(40), provider: 'railway', evidenceId: 'receipted' };

function fixture() {
  return {
    component: 'api',
    serviceId: 'api-service',
    selected: { api: true },
    receipts: [receipt],
    liveDeploymentId: 'redeployed',
    history: [
      { id: 'redeployed', status: 'SUCCESS', createdAt: '2026-10-05T00:02:11Z',
        meta: { reason: 'redeploy', imageDigest: digest } },
      { id: 'receipted', status: 'REMOVED', createdAt: '2026-10-04T07:39:00Z',
        meta: { reason: 'deploy', imageDigest: digest } },
      { id: 'earlier', status: 'REMOVED', createdAt: '2026-10-02T07:33:13Z',
        meta: { reason: 'deploy', imageDigest: `sha256:${'c'.repeat(64)}` } },
    ],
  };
}

test('a same-image redeploy replaces the receipt for a release that deploys it', () => {
  assert.deepEqual(verifyRedeployedReplacement(fixture()), { ...receipt, evidenceId: 'redeployed' });
});

test('a same-image Railway failover replaces the receipt like a redeploy', () => {
  const failover = fixture();
  failover.history[0].meta.reason = 'failover';
  assert.deepEqual(verifyRedeployedReplacement(failover), { ...receipt, evidenceId: 'redeployed' });

  const otherImage = fixture();
  otherImage.history[0].meta = { reason: 'failover', imageDigest: `sha256:${'d'.repeat(64)}` };
  assert.throws(() => verifyRedeployedReplacement(otherImage), unreceiptedChange);
});

test('a Railway rollback to an earlier receipted image restores that receipt', () => {
  const earlierReceipt = { sourceSha: 'f'.repeat(40), provider: 'railway', evidenceId: 'earlier' };
  const rolledBack = fixture();
  rolledBack.receipts.push(earlierReceipt);
  rolledBack.history[0].meta = { reason: 'rollback', imageDigest: `sha256:${'c'.repeat(64)}` };
  assert.deepEqual(verifyRedeployedReplacement(rolledBack), { ...earlierReceipt, evidenceId: 'redeployed' });

  const unreceipted = fixture();
  unreceipted.history[0].meta = { reason: 'rollback', imageDigest: `sha256:${'c'.repeat(64)}` };
  assert.throws(() => verifyRedeployedReplacement(unreceipted), unreceiptedChange);
});

function variablePatch(overrides = {}) {
  const deployed = fixture();
  deployed.history[0].meta = { reason: 'deploy', imageDigest: digest, patchId: 'patch' };
  const patch = { status: 'COMMITTED',
    patch: { services: { 'api-service': { variables: { SCOPE_API_PUBLIC_URL: { value: '*****' } } } } },
    ...overrides };
  deployed.readPatch = id => (id === 'patch' ? patch : null);
  return deployed;
}

test('a same-image deploy from a committed variable patch replaces the receipt', () => {
  assert.deepEqual(verifyRedeployedReplacement(variablePatch()), { ...receipt, evidenceId: 'redeployed' });
});

test('a same-image deploy replaces the receipt only for a committed patch of variables alone', () => {
  assert.throws(() => verifyRedeployedReplacement(variablePatch({ status: 'STAGED' })), unreceiptedChange);
  assert.throws(() => verifyRedeployedReplacement(variablePatch({ patch: { services: {
    'api-service': { variables: {}, deploy: { startCommand: 'other' } } } } })), unreceiptedChange);
  assert.throws(() => verifyRedeployedReplacement(variablePatch({ patch: { services: {
    'other-service': { variables: {} } } } })), unreceiptedChange);
  assert.throws(() => verifyRedeployedReplacement(variablePatch({ patch: { services: {
    'api-service': { variables: {} } }, volumes: {} } })), unreceiptedChange);

  const unpatched = variablePatch();
  delete unpatched.history[0].meta.patchId;
  assert.throws(() => verifyRedeployedReplacement(unpatched), unreceiptedChange);

  const otherImage = variablePatch();
  otherImage.history[0].meta.imageDigest = `sha256:${'d'.repeat(64)}`;
  assert.throws(() => verifyRedeployedReplacement(otherImage), unreceiptedChange);
});

test('the release must deploy the redeployed component', () => {
  assert.throws(() => verifyRedeployedReplacement({ ...fixture(), selected: { api: false } }),
    /does not replace api/);
  assert.throws(() => verifyRedeployedReplacement({ ...fixture(), component: 'runner-image' }),
    /not a Railway service/);
});

test('the live deployment must run the receipted image through redeploys only', () => {
  const otherImage = fixture();
  otherImage.history[0].meta.imageDigest = `sha256:${'d'.repeat(64)}`;
  assert.throws(() => verifyRedeployedReplacement(otherImage), unreceiptedChange);

  const intervening = fixture();
  intervening.history.splice(1, 0, { id: 'manual', status: 'REMOVED', createdAt: '2026-10-04T12:00:00Z',
    meta: { reason: 'deploy', imageDigest: digest } });
  assert.throws(() => verifyRedeployedReplacement(intervening), unreceiptedChange);

  assert.throws(() => verifyRedeployedReplacement({ ...fixture(), liveDeploymentId: 'receipted' }),
    /not a successful redeploy/);
});

test('the receipted deployment must be superseded and carry an exact digest', () => {
  const active = fixture();
  active.history[1].status = 'SUCCESS';
  assert.throws(() => verifyRedeployedReplacement(active), /has not been superseded/);

  const pinned = fixture();
  pinned.receipts = [{ ...receipt, artifactDigest: `sha256:${'e'.repeat(64)}` }];
  assert.throws(() => verifyRedeployedReplacement(pinned), /no matching image digest/);
});

test('loading reads the production service history and returns the replaced receipt', () => {
  const { history, selected } = variablePatch();
  const patch = variablePatch().readPatch('patch');
  const manifest = { railway: { projectId: 'project' }, services: { api: { id: 'api-service' } },
    environments: { production: { environmentId: 'production' } } };
  const status = { environments: { edges: [{ node: { id: 'production', serviceInstances: { edges: [
    { node: { serviceId: 'api-service', activeDeployments: [{ id: 'redeployed', status: 'SUCCESS' }] } },
  ] } } }] } };
  const reads = [];
  const result = loadRedeployedReplacement({ component: 'api', selected, receipts: [receipt],
    status, manifest, railway: (args, options) => {
      reads.push({ args, options });
      return args[0] === 'api' ? { data: { environmentPatch: patch } } : history;
    } });
  assert.deepEqual(result, { api: { ...receipt, evidenceId: 'redeployed' } });
  assert.deepEqual(reads[0], { args: ['deployment', 'list', '--project', 'project', '--service', 'api-service',
    '--environment', 'production', '--limit', '100', '--json'], options: undefined });
  assert.equal(reads.length, 2);
  assert.match(reads[1].args[1], /^query EnvironmentPatch\b/);
  assert.deepEqual({ ...reads[1], args: reads[1].args.toSpliced(1, 1) },
    { args: ['api', '--variables', '@-'], options: { input: '{"id":"patch"}' } });
});
