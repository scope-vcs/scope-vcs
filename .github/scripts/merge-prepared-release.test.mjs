import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { mergePreparedReleaseFragments } from './merge-prepared-release.mjs';

const sourceSha = 'a'.repeat(40);
const services = JSON.parse(readFileSync(new URL('../deployment-services.json', import.meta.url), 'utf8')).services;
const metadata = { schemaVersion: 1, sourceSha, preparationRunId: '1234', maintenanceSha256: 'b'.repeat(64) };
function fragment(component, overrides = {}) {
  return {
    ...metadata,
    components: {
      [component]: {
        sourceSha,
        image: `ghcr.io/scope-vcs/scope-vcs/railway-private-${component}@sha256:${'c'.repeat(64)}`,
        serviceId: services[component].id,
      },
    },
    ...overrides,
  };
}
const options = { sourceSha, components: ['api', 'run-worker'], services };

test('merges verified singleton fragments with exact digest and release metadata', () => {
  const api = fragment('api');
  const worker = fragment('run-worker');
  const result = mergePreparedReleaseFragments([api, worker], options);
  assert.deepEqual(result.components, { ...api.components, ...worker.components });
  assert.equal(result.preparationRunId, '1234');
  assert.equal(result.maintenanceSha256, metadata.maintenanceSha256);
});

test('rejects omitted, duplicate, and multi-component fragments', () => {
  assert.throws(() => mergePreparedReleaseFragments([fragment('api')], options), /one prepared fragment/);
  assert.throws(() => mergePreparedReleaseFragments([fragment('api'), fragment('api')], options), /duplicate/);
  assert.throws(() => mergePreparedReleaseFragments([
    { ...fragment('api'), components: { ...fragment('api').components, ...fragment('run-worker').components } },
    fragment('run-worker'),
  ], options), /exactly one/);
});

test('rejects mismatched provenance and release metadata before publishing aggregate', () => {
  const api = fragment('api');
  const worker = fragment('run-worker');
  assert.throws(() => mergePreparedReleaseFragments([api, { ...worker, sourceSha: 'd'.repeat(40) }], options), /revision/);
  assert.throws(() => mergePreparedReleaseFragments([api, { ...worker, preparationRunId: '9999' }], options), /disagree/);
  assert.throws(() => mergePreparedReleaseFragments([api, { ...worker, maintenanceSha256: 'd'.repeat(64) }], options), /disagree/);
  assert.throws(() => mergePreparedReleaseFragments([api, fragment('run-worker', { components: { 'run-worker': { ...worker.components['run-worker'], serviceId: 'wrong' } } })], options), /wrong service/);
  assert.throws(() => mergePreparedReleaseFragments([{ ...api, maintenanceSha256: undefined }, { ...worker, maintenanceSha256: undefined }], options), /maintenance binary digest/);
});

test('CLI publishes one aggregate only for the current preparation run', () => {
  const directory = mkdtempSync(join(tmpdir(), 'scope-release-merge-'));
  try {
    const output = join(directory, 'prepared-release.json');
    const api = join(directory, 'api.json');
    const worker = join(directory, 'run-worker.json');
    writeFileSync(api, JSON.stringify(fragment('api')));
    writeFileSync(worker, JSON.stringify(fragment('run-worker')));
    const command = new URL('./merge-prepared-release.mjs', import.meta.url).pathname;
    const args = [command, output, sourceSha, 'api run-worker', api, worker];
    const run = runId => spawnSync(process.execPath, args, {
      env: { ...process.env, GITHUB_RUN_ID: runId }, encoding: 'utf8',
    });
    assert.notEqual(run('9999').status, 0);
    assert.throws(() => readFileSync(output), /ENOENT/);
    const accepted = run('1234');
    assert.equal(accepted.status, 0, accepted.stderr);
    assert.equal(JSON.parse(readFileSync(output, 'utf8')).preparationRunId, '1234');
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
