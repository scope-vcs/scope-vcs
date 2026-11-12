import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, truncateSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { backendBinaryCaps, backendPrebuiltComponents, buildBackendComponent } from './deployment-components.mjs';

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

function check(t, sizes, args = []) {
  const root = mkdtempSync(join(tmpdir(), 'scope-binary-sizes-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  mkdirSync(join(root, 'bin'));
  for (const [name, bytes] of Object.entries(sizes)) {
    writeFileSync(join(root, 'bin', name), '');
    truncateSync(join(root, 'bin', name), bytes);
  }
  writeFileSync(join(root, 'manifest.json'), JSON.stringify(manifest));
  const summary = join(root, 'summary.md');
  const result = spawnSync(process.execPath, [script, join(root, 'bin'), ...args], {
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

test('a component check covers only that component binaries', t => {
  const passed = check(t, { 'scope-api': 100, 'scope-maintenance': 50 }, ['--component', 'api']);
  assert.equal(passed.status, 0, passed.stderr);
  assert.doesNotMatch(passed.summary, /scope-worker/);
  const missing = check(t, { 'scope-api': 100 }, ['--component', 'api']);
  assert.equal(missing.status, 1);
  assert.deepEqual(missing.stderr.trim().split('\n'), ['scope-maintenance was not built']);
  const foreign = check(t, { 'scope-worker': 1 }, ['--component', 'api']);
  assert.equal(foreign.status, 1);
  assert.match(foreign.stderr, /scope-worker has no size cap for component api/);
});

test('each backend component builds in its own cargo selection so shared crates keep its feature set', t => {
  const root = mkdtempSync(join(tmpdir(), 'scope-backend-build-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const builds = [];
  const run = (program, args) => {
    assert.equal(program, 'cargo');
    if (args[0] === 'metadata') return JSON.stringify({ target_directory: join(root, 'target') });
    builds.push(args);
    mkdirSync(join(root, 'target/release'), { recursive: true });
    for (let index = args.indexOf('--bin'); index !== -1; index = args.indexOf('--bin', index + 1)) {
      writeFileSync(join(root, 'target/release', args[index + 1]), args[index + 1]);
    }
  };
  for (const component of backendPrebuiltComponents()) buildBackendComponent(component, join(root, 'bin'), { run, root });
  assert.deepEqual(builds.map(args => args.filter(arg => arg !== '--bin').slice(4)), [
    ['worker/Cargo.toml', 'worker'],
    ['repo-router/Cargo.toml', 'scope-repo-router'],
    ['media-service/Cargo.toml', 'scope-media-service'],
    ['api/Cargo.toml', 'api', 'scope-maintenance'],
  ]);
  assert.ok(builds.every(args => !args.includes('-p') && !args.includes('--workspace')));
  assert.equal(readFileSync(join(root, 'bin/scope-vcs'), 'utf8'), 'api');
  assert.equal(statSync(join(root, 'bin/scope-worker')).mode & 0o777, 0o755);
  assert.deepEqual(readdirSync(join(root, 'bin')).sort(), backendBinaryCaps().map(({ binary }) => binary).sort());
});

test('caps must be positive integers', () => {
  for (const maxBytes of [undefined, 0, -1, 1.5, '100']) {
    const invalid = structuredClone(manifest);
    invalid.services.worker.deployment.artifact.maxBytes = maxBytes;
    assert.throws(() => backendBinaryCaps(invalid), /Component worker has no positive integer maxBytes for scope-worker/);
  }
});
