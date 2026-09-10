import assert from 'node:assert/strict';
import test from 'node:test';
import { execFileSync } from 'node:child_process';
import { accessSync, constants, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
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

test('routine staging deploys once and transition tests are opt-in', () => {
  const workflow = readFileSync(new URL('../workflows/deploy-staging.yml', import.meta.url), 'utf8');
  assert.match(workflow, /name: Exercise optional release transitions\n        if: inputs.run_transition_tests/);
  assert.equal((workflow.match(/run: bash \.\.\/\.github\/scripts\/deploy-staging-railway\.sh/g) ?? []).length, 1);
  assert.doesNotMatch(workflow, /staging-smoke-seed\.sh seed|cargo build[^\n]+scope-maintenance|pnpm build/);
  assert.match(workflow, /staging-smoke-seed\.sh grant/);
});
