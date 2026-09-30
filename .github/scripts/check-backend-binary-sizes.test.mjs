import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, truncateSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { backendBinaryCaps } from './deployment-components.mjs';

const script = fileURLToPath(new URL('./check-backend-binary-sizes.mjs', import.meta.url));
const binary = (name, maxBytes, extra = {}) => ({ kind: 'binary', binary: name, maxBytes, ...extra });
const manifest = {
  services: {
    api: { deployment: { backend: true, artifact: binary('scope-api', 100, { maintenance: { binary: 'scope-maintenance', maxBytes: 50 } }) } },
    worker: { deployment: { backend: true, artifact: binary('scope-worker', 200) } },
    'media-worker': { deployment: { backend: true, artifact: { kind: 'external-image', binary: 'scope-media-worker' } } },
    'cli-downloads': { deployment: { backend: false, artifact: binary('scope-cli-service') } },
  },
};

function check(t, sizes) {
  const root = mkdtempSync(join(tmpdir(), 'scope-binary-sizes-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  mkdirSync(join(root, 'bin'));
  for (const [name, bytes] of Object.entries(sizes)) {
    writeFileSync(join(root, 'bin', name), '');
    truncateSync(join(root, 'bin', name), bytes);
  }
  writeFileSync(join(root, 'manifest.json'), JSON.stringify(manifest));
  const summary = join(root, 'summary.md');
  const result = spawnSync(process.execPath, [script, join(root, 'bin')], {
    encoding: 'utf8',
    env: { ...process.env, SCOPE_DEPLOYMENT_MANIFEST: join(root, 'manifest.json'), GITHUB_STEP_SUMMARY: summary },
  });
  return { ...result, summary: readFileSync(summary, 'utf8') };
}

test('binaries below or exactly at their caps pass and are summarized', t => {
  const result = check(t, { 'scope-api': 99, 'scope-maintenance': 50, 'scope-worker': 200 });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.summary, /\| `scope-maintenance` \| 50 \| 0\.0 \| 50 \| 0 \|/);
  assert.match(result.summary, /\| `scope-worker` \| 200 \| 0\.0 \| 200 \| 0 \|/);
});

test('every oversized binary is reported with its bytes, cap and excess', t => {
  const result = check(t, { 'scope-api': 100, 'scope-maintenance': 51, 'scope-worker': 260 });
  assert.equal(result.status, 1);
  assert.deepEqual(result.stderr.trim().split('\n'), [
    'scope-maintenance is 51 bytes, over the 50 byte cap by 1 bytes',
    'scope-worker is 260 bytes, over the 200 byte cap by 60 bytes',
  ]);
  assert.match(result.summary, /\| `scope-worker` \| 260 \| 0\.0 \| 200 \| 60 \|/);
});

test('built and capped binaries must match', t => {
  const result = check(t, { 'scope-api': 1, 'scope-maintenance': 1, 'scope-extra': 1 });
  assert.equal(result.status, 1);
  assert.deepEqual(result.stderr.trim().split('\n'), [
    'scope-extra has no size cap in the deployment manifest',
    'scope-worker was not built',
  ]);
});

test('caps must be positive integers', () => {
  for (const maxBytes of [undefined, 0, -1, 1.5, '100']) {
    const invalid = structuredClone(manifest);
    invalid.services.worker.deployment.artifact.maxBytes = maxBytes;
    assert.throws(() => backendBinaryCaps(invalid), /Component worker has no positive integer maxBytes for scope-worker/);
  }
});

test('the deployment manifest caps all six backend release binaries', () => {
  assert.deepEqual(backendBinaryCaps().map(({ binary }) => binary).sort(), [
    'scope-cache-service', 'scope-maintenance', 'scope-media-service', 'scope-repo-router', 'scope-vcs', 'scope-worker',
  ]);
});
