import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { test } from 'node:test';

test('closed and unlabeled pull requests wake trusted reconciliation without pull_request_target or PR payloads', () => {
  const workflows = JSON.parse(execFileSync('python3', ['-c',
    [
      'import json, pathlib, yaml',
      "paths = [pathlib.Path('.github/workflows/' + name + '.yml') for name in ['preview-environment', 'preview-environment-signal', 'audit-railway-experiments']]",
      'print(json.dumps({path.stem: yaml.load(path.read_text(), Loader=yaml.BaseLoader) for path in paths if path.exists()}))',
    ].join('\n'),
  ], { encoding: 'utf8' }));
  const deployment = workflows['preview-environment'];
  assert.equal('pull_request_target' in deployment.on, false);
  const signal = workflows['preview-environment-signal'];
  assert.deepEqual(signal.on, { pull_request: { types: ['closed', 'unlabeled'] } });
  assert.deepEqual(signal.permissions, {});
  const notification = signal.jobs.signal;
  const shouldSignal = Function('github', `return ${notification.if};`);
  for (const [event, expected] of [
    [{ action: 'closed' }, true],
    [{ action: 'unlabeled', label: { name: 'preview' } }, true],
    [{ action: 'unlabeled', label: { name: 'other' } }, false],
  ]) {
    assert.equal(shouldSignal({ event }), expected);
  }
  const sweep = workflows['audit-railway-experiments'];
  assert.deepEqual(sweep.on.workflow_run, { workflows: [signal.name], types: ['completed'] });
  assert.ok(sweep.on.schedule.length > 0);
  assert.equal('pull_request_target' in sweep.on, false);
  const shouldReconcile = Function('github', `return ${sweep.jobs.reconcile.if};`);
  for (const event_name of ['workflow_run', 'schedule']) {
    assert.equal(shouldReconcile({ event_name, ref: 'refs/heads/main' }), true);
  }
  const checkout = sweep.jobs.reconcile.steps.find(step => step.uses?.startsWith('actions/checkout@'));
  assert.equal(checkout.with.ref, 'main');
  assert.match(sweep.jobs.reconcile.steps.find(step => step.id === 'plan').run, /preview-pull-request\.mjs reconcile/);
  assert.equal(sweep.jobs.cleanup.needs, 'reconcile');
  assert.match(sweep.jobs.cleanup.strategy.matrix.pull_request, /needs\.reconcile\.outputs\.pull_requests/);
});
