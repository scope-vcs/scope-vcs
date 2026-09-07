import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const repository = fileURLToPath(new URL('../..', import.meta.url));
const extractor = join(repository, '.github/scripts/extract-railway-maintenance.sh');
const preparer = join(repository, '.github/scripts/prepare-railway-artifact.sh');
const sourceSha = 'a'.repeat(40);
const digest = `sha256:${'b'.repeat(64)}`;
const image = `ghcr.io/example/release/railway-api@${digest}`;
const serviceId = JSON.parse(readFileSync(join(repository, '.github/deployment-services.json'))).services.api.id;
const fakeDocker = `#!/usr/bin/env node
const fs = require('node:fs');
const path = require('node:path');
const args = process.argv.slice(2);
fs.appendFileSync(process.env.DOCKER_TEST_LOG, JSON.stringify({ args, config: process.env.DOCKER_CONFIG }) + '\\n');
if (args[0] === 'login') {
  const password = fs.readFileSync(0, 'utf8');
  if (password !== process.env.SCOPE_RAILWAY_REGISTRY_PASSWORD) process.exit(2);
  fs.writeFileSync(path.join(process.env.DOCKER_CONFIG, 'config.json'), 'private login');
} else if (args[0] === 'buildx') {
  const context = args.at(-1);
  fs.copyFileSync(path.join(context, 'bin/scope-maintenance'), process.env.DOCKER_TEST_IMAGE_BINARY);
  fs.writeFileSync(args[args.indexOf('--metadata-file') + 1], JSON.stringify({ 'containerimage.digest': process.env.DOCKER_TEST_DIGEST }));
} else if (args[0] === 'manifest' || args[0] === 'pull') {
  if (process.env.DOCKER_TEST_PULL_FAIL === '1') process.exit(3);
} else if (args[0] === 'create') {
  if (!args.includes('--entrypoint') || args[args.indexOf('--entrypoint') + 1] !== '/bin/false') process.exit(4);
} else if (args[0] === 'cp') {
  if (!args[1].endsWith(':/app/bin/scope-maintenance')) process.exit(5);
  if (process.env.DOCKER_TEST_COPY_SYMLINK === '1') fs.symlinkSync(process.env.DOCKER_TEST_IMAGE_BINARY, args[2]);
  else fs.copyFileSync(process.env.DOCKER_TEST_IMAGE_BINARY, args[2]);
} else if (args[0] !== 'container' || args[1] !== 'rm') {
  process.stderr.write('Forbidden Docker operation: ' + args.join(' '));
  process.exit(6);
}
`;
function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'scope-maintenance-image-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const bin = join(root, 'tools');
  mkdirSync(bin);
  writeFileSync(join(bin, 'docker'), fakeDocker, { mode: 0o755 });
  const imageBinary = join(root, 'registry-image-maintenance');
  writeFileSync(imageBinary, 'original compiled maintenance binary');
  const inheritedConfig = join(root, 'publishing-login');
  mkdirSync(inheritedConfig);
  writeFileSync(join(inheritedConfig, 'config.json'), 'unrelated publishing credentials');
  const manifest = join(root, 'prepared-release.json');
  const release = {
    schemaVersion: 1, sourceSha,
    maintenanceSha256: createHash('sha256').update(readFileSync(imageBinary)).digest('hex'),
    components: { api: { sourceSha, image, serviceId } },
  };
  writeFileSync(manifest, JSON.stringify(release));
  const log = join(root, 'docker.ndjson');
  const env = {
    ...process.env, PATH: `${bin}:${process.env.PATH}`, DOCKER_CONFIG: inheritedConfig,
    DOCKER_TEST_LOG: log, DOCKER_TEST_IMAGE_BINARY: imageBinary, DOCKER_TEST_DIGEST: digest,
    SCOPE_DEPLOYMENT_SOURCE_SHA: sourceSha,
    SCOPE_RAILWAY_REGISTRY_USERNAME: '', SCOPE_RAILWAY_REGISTRY_PASSWORD: '',
  };
  return { root, manifest, release, imageBinary, env, inheritedConfig, log, destination: join(root, 'target/release/scope-maintenance') };
}
function commands(f) {
  return existsSync(f.log) ? readFileSync(f.log, 'utf8').trim().split('\n').map(JSON.parse) : [];
}
function extract(f, extra = {}) {
  return spawnSync('bash', [extractor, f.manifest, f.destination], { cwd: repository, env: { ...f.env, ...extra }, encoding: 'utf8' });
}

test('ordinary deployment and later recovery use the pinned image after build artifacts expire', (t) => {
  const f = fixture(t);
  for (const attempt of ['ordinary', 'recovery']) {
    const result = extract(f, attempt === 'recovery' ? { SCOPE_RECOVER_CLOSED_CUTOVER: '1' } : {});
    assert.equal(result.status, 0, result.stderr);
    assert.equal(readFileSync(f.destination, 'utf8'), readFileSync(f.imageBinary, 'utf8'));
    assert.equal(statSync(f.destination).mode & 0o777, 0o755);
  }
  const calls = commands(f);
  assert.equal(calls.filter(({ args }) => args[0] === 'pull').length, 2);
  assert.ok(calls.filter(({ args }) => args[0] === 'pull' || args[0] === 'create').every(({ args }) => args.at(-1) === image));
  assert.ok(calls.every(({ args }) => !['start', 'run', 'exec', 'buildx'].includes(args[0])));
  assert.equal(calls.filter(({ args }) => args[0] === 'container' && args[1] === 'rm').length, 2);
  assert.ok(calls.every(({ config }) => config !== f.inheritedConfig && !existsSync(config)));
  assert.equal(readFileSync(join(f.inheritedConfig, 'config.json'), 'utf8'), 'unrelated publishing credentials');
});

test('uses scoped durable registry credentials through stdin and removes them after extraction', (t) => {
  const f = fixture(t);
  const password = 'test durable pull credential';
  const result = extract(f, { SCOPE_RAILWAY_REGISTRY_USERNAME: 'pull-user', SCOPE_RAILWAY_REGISTRY_PASSWORD: password });
  assert.equal(result.status, 0, result.stderr);
  const login = commands(f).find(({ args }) => args[0] === 'login');
  assert.deepEqual(login.args, ['login', 'ghcr.io', '--username', 'pull-user', '--password-stdin']);
  assert.equal(existsSync(login.config), false);
  assert.ok(!readFileSync(f.log, 'utf8').includes(password));
  assert.ok(!result.stdout.includes(password));
});

test('hash mismatch and symlink copies leave an existing maintenance binary untouched', (t) => {
  for (const mode of ['wrong-hash', 'symlink']) {
    const f = fixture(t);
    mkdirSync(dirname(f.destination), { recursive: true });
    writeFileSync(f.destination, 'previous verified binary');
    if (mode === 'wrong-hash') writeFileSync(f.imageBinary, 'different maintenance binary');
    const result = extract(f, { DOCKER_TEST_COPY_SYMLINK: mode === 'symlink' ? '1' : '' });
    assert.notEqual(result.status, 0);
    assert.equal(readFileSync(f.destination, 'utf8'), 'previous verified binary');
    assert.equal(commands(f).at(-1).args[0], 'container');
    assert.ok(commands(f).every(({ config }) => !existsSync(config)));
  }
});

test('fails before Docker access for mutable image, missing hash, wrong source, or partial credentials', (t) => {
  for (const failure of ['mutable', 'missing-hash', 'source', 'credentials']) {
    const f = fixture(t);
    if (failure === 'mutable') f.release.components.api.image = 'ghcr.io/example/release/railway-api:latest';
    if (failure === 'missing-hash') delete f.release.maintenanceSha256;
    writeFileSync(f.manifest, JSON.stringify(f.release));
    const result = extract(f, {
      ...(failure === 'source' ? { SCOPE_DEPLOYMENT_SOURCE_SHA: 'c'.repeat(40) } : {}),
      ...(failure === 'credentials' ? { SCOPE_RAILWAY_REGISTRY_USERNAME: 'partial' } : {}),
    });
    assert.notEqual(result.status, 0);
    assert.equal(commands(f).length, 0);
  }
});

test('API preparation embeds and binds the original maintenance binary before publishing', (t) => {
  const f = fixture(t);
  rmSync(f.manifest);
  const context = join(f.root, 'api-context');
  mkdirSync(join(context, 'bin'), { recursive: true });
  writeFileSync(join(context, 'bin/scope-vcs'), 'api binary', { mode: 0o755 });
  writeFileSync(join(context, 'bin/scope-maintenance'), readFileSync(f.imageBinary));
  const original = join(f.root, 'build-artifact-maintenance');
  writeFileSync(original, readFileSync(f.imageBinary));
  const result = spawnSync('bash', [preparer, 'api', context, f.manifest], {
    cwd: repository, encoding: 'utf8', env: { ...f.env, GITHUB_REPOSITORY: 'example/release', SCOPE_MAINTENANCE_BINARY: original },
  });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(JSON.parse(readFileSync(f.manifest)).maintenanceSha256, f.release.maintenanceSha256);
  rmSync(original);
  rmSync(context, { recursive: true });
  assert.equal(extract(f).status, 0);
});

test('API preparation rejects a different embedded maintenance binary before publishing', (t) => {
  const f = fixture(t);
  const context = join(f.root, 'api-context');
  mkdirSync(join(context, 'bin'), { recursive: true });
  writeFileSync(join(context, 'bin/scope-vcs'), 'api binary', { mode: 0o755 });
  writeFileSync(join(context, 'bin/scope-maintenance'), 'wrong binary');
  const result = spawnSync('bash', [preparer, 'api', context, f.manifest], {
    cwd: repository, encoding: 'utf8', env: { ...f.env, GITHUB_REPOSITORY: 'example/release', SCOPE_MAINTENANCE_BINARY: f.imageBinary },
  });
  assert.notEqual(result.status, 0);
  assert.equal(commands(f).length, 0);
});
