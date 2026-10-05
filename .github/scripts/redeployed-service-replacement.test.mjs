import assert from 'node:assert/strict';
import test from 'node:test';
import { loadRedeployedReplacement, verifyRedeployedReplacement } from './redeployed-service-replacement.mjs';

const digest = `sha256:${'a'.repeat(64)}`;
const receipt = { sourceSha: 'b'.repeat(40), provider: 'railway', evidenceId: 'receipted' };

function fixture() {
  return {
    component: 'api',
    selected: { api: true },
    receipt,
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

test('the release must deploy the redeployed component', () => {
  assert.throws(() => verifyRedeployedReplacement({ ...fixture(), selected: { api: false } }),
    /does not replace api/);
  assert.throws(() => verifyRedeployedReplacement({ ...fixture(), component: 'runner-image' }),
    /not a Railway service/);
});

test('the live deployment must run the receipted image through redeploys only', () => {
  const otherImage = fixture();
  otherImage.history[0].meta.imageDigest = `sha256:${'d'.repeat(64)}`;
  assert.throws(() => verifyRedeployedReplacement(otherImage), /not a redeploy of the receipted image/);

  const intervening = fixture();
  intervening.history.splice(1, 0, { id: 'manual', status: 'REMOVED', createdAt: '2026-10-04T12:00:00Z',
    meta: { reason: 'deploy', imageDigest: digest } });
  assert.throws(() => verifyRedeployedReplacement(intervening), /not a redeploy of the receipted image/);

  assert.throws(() => verifyRedeployedReplacement({ ...fixture(), liveDeploymentId: 'receipted' }),
    /not a successful redeploy/);
});

test('the receipted deployment must be superseded and carry an exact digest', () => {
  const active = fixture();
  active.history[1].status = 'SUCCESS';
  assert.throws(() => verifyRedeployedReplacement(active), /has not been superseded/);

  const pinned = fixture();
  pinned.receipt = { ...receipt, artifactDigest: `sha256:${'e'.repeat(64)}` };
  assert.throws(() => verifyRedeployedReplacement(pinned), /no matching image digest/);
});

test('loading reads the production service history and returns the replaced receipt', () => {
  const { history, selected } = fixture();
  const manifest = { railway: { projectId: 'project' }, services: { api: { id: 'api-service' } },
    environments: { production: { environmentId: 'production' } } };
  const status = { environments: { edges: [{ node: { id: 'production', serviceInstances: { edges: [
    { node: { serviceId: 'api-service', activeDeployments: [{ id: 'redeployed', status: 'SUCCESS' }] } },
  ] } } }] } };
  let args;
  const result = loadRedeployedReplacement({ component: 'api', selected, deployments: { api: receipt },
    status, manifest, railway: (value) => { args = value; return history; } });
  assert.deepEqual(result, { api: { ...receipt, evidenceId: 'redeployed' } });
  assert.deepEqual(args, ['deployment', 'list', '--project', 'project', '--service', 'api-service',
    '--environment', 'production', '--limit', '100', '--json']);
});
