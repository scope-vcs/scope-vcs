import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { baselineIdentity, sameBaseline } from './staging-baseline.mjs';

test('baseline identity follows applied schema independently of candidate migrations', () => {
  const current = { applied: ['m0001_initial', 'm0002_repositories'], pending: [] };
  const candidate = { ...current, pending: [{ name: 'm0003_media' }] };
  assert.equal(sameBaseline(current, candidate), true);
  assert.equal(sameBaseline(current, { applied: ['m0001_initial'] }), false);
  assert.equal(sameBaseline(current, { applied: [...current.applied, 'm0003_media'] }), false);
});

test('unknown and empty baselines cannot select a retained snapshot', () => {
  for (const applied of [undefined, [], ['m0001_initial', 'm0001_initial'], ['../snapshot']]) {
    assert.throws(() => baselineIdentity({ applied }));
  }
});

test('staging workflows scope the encryption key to baseline handling and retain only ciphertext', () => {
  const workflow = readFileSync(new URL('../workflows/deploy-staging.yml', import.meta.url), 'utf8');
  const baselineStep = workflow.split('      - name: Reconcile and snapshot staging baseline\n')[1].split('\n      - name:')[0];
  assert.match(baselineStep, /SCOPE_STAGING_BASELINE_KEY: \$\{\{ secrets\.SCOPE_STAGING_BASELINE_KEY \}\}/);
  assert.equal(workflow.replace(baselineStep, '').includes('SCOPE_STAGING_BASELINE_KEY'), false);
  const provisioning = readFileSync(new URL('../workflows/provision-staging-baseline.yml', import.meta.url), 'utf8');
  const restoreStep = provisioning.split('      - name: Restore retained staging baseline\n')[1].split('\n      - name:')[0];
  assert.match(restoreStep, /SCOPE_STAGING_BASELINE_KEY: \$\{\{ secrets\.SCOPE_STAGING_BASELINE_KEY \}\}/);
  assert.equal(provisioning.replace(restoreStep, '').includes('SCOPE_STAGING_BASELINE_KEY'), false);
  const uploadStep = workflow.split('      - name: Retain staging baseline before migration\n')[1].split('\n      - name:')[0];
  const paths = uploadStep.split('          path: |\n')[1].split('          retention-days:')[0].trim().split('\n').map(line => line.trim());
  assert.deepEqual(paths, [
    '${{ runner.temp }}/staging-baseline/database.dump.enc',
    '${{ runner.temp }}/staging-baseline/database.sha256',
    '${{ runner.temp }}/staging-baseline/baseline.json',
  ]);
});

test('provisioning recognizes unclaimed source blob cleanup by the queue grace period', () => {
  const script = readFileSync(new URL('./staging-deletion-window.sh', import.meta.url), 'utf8');
  const queue = readFileSync(new URL('../../crates/scope-postgres/src/db/cleanup_queue/queue.rs', import.meta.url), 'utf8');
  const grace = queue.match(/const SOURCE_BLOB_DELETE_GRACE_SECONDS: u64 = (\d+);/)?.[1];
  assert.ok(grace);
  assert.match(script, new RegExp(`^source_blob_queue_delay_seconds=${grace}$`, 'm'));
});

test('staging postpones migration deletions beyond the retained baseline', () => {
  const deletionWindowScript = readFileSync(new URL('./staging-deletion-window.sh', import.meta.url), 'utf8');
  const seconds = Number(deletionWindowScript.match(/^staging_deletion_window_seconds=\$\(\((\d+) \* 24 \* 60 \* 60\)\)$/m)?.[1]) * 24 * 60 * 60;
  const workflow = readFileSync(new URL('../workflows/deploy-staging.yml', import.meta.url), 'utf8');
  const uploadStep = workflow.split('      - name: Retain staging baseline before migration\n')[1].split('\n      - name:')[0];
  const retentionDays = Number(uploadStep.match(/retention-days: (\d+)/)?.[1]);
  assert.ok(retentionDays > 0);
  assert.ok(seconds > retentionDays * 24 * 60 * 60);
});
