import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
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

test('candidate preparation precedes the job that can stop staging writers', () => {
  const workflow = readFileSync(new URL('../workflows/scope-railway-staging.yml', import.meta.url), 'utf8');
  const proof = workflow.slice(workflow.indexOf('\n  prove:'));
  assert.match(proof, /needs: \[prepare, prepare-images\]/);
  assert(proof.indexOf('Close staging writers') < proof.indexOf('Checkout exact candidate revision'));
  assert.doesNotMatch(proof, /cargo build/);
  assert.match(proof, /ref: \$\{\{ github\.sha \}\}\n\s+persist-credentials: false/);
  assert.match(proof, /working-directory: candidate[\s\S]+rehearse-release\.mjs/);
});
