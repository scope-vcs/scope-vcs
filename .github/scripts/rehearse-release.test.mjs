import assert from 'node:assert/strict';
import test from 'node:test';
import { previousDeploymentsRemoved } from './rehearse-release.mjs';

test('teardown proof requires every exact predecessor to be removed', () => {
  const previous = [{ serviceId: 'api', deploymentId: 'old-api' }, { serviceId: 'web', deploymentId: 'old-web' }];
  const deployments = {
    api: [{ id: 'old-api', status: 'REMOVED' }, { id: 'new-api', status: 'SUCCESS' }],
    web: [{ id: 'old-web', status: 'REMOVING' }, { id: 'new-web', status: 'SUCCESS' }],
  };
  assert.equal(previousDeploymentsRemoved(previous, deployments), false);
  deployments.web[0].status = 'REMOVED';
  assert.equal(previousDeploymentsRemoved(previous, deployments), true);
  deployments.web.shift();
  assert.equal(previousDeploymentsRemoved(previous, deployments), false);
});
