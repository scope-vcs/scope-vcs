import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const manifest = JSON.parse(readFileSync(new URL('../deployment-services.json', import.meta.url)));
const workflow = readFileSync(new URL('../workflows/scope-railway-staging.yml', import.meta.url), 'utf8');
const seedScript = fileURLToPath(new URL('./staging-smoke-seed.sh', import.meta.url));

function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'scope-smoke-setup-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  mkdirSync(join(root, 'bin'));
  mkdirSync(join(root, 'credentials'), { mode: 0o700 });
  const manifestPath = join(root, 'manifest.json');
  writeFileSync(manifestPath, JSON.stringify(manifest));
  writeFileSync(join(root, 'bin/railway'), `#!/usr/bin/env node
const { readFileSync, appendFileSync } = require('node:fs');
const { spawnSync } = require('node:child_process');
const m = JSON.parse(readFileSync(process.env.SCOPE_DEPLOYMENT_MANIFEST));
const args = process.argv.slice(2);
const project = args[args.indexOf('--project') + 1];
const environment = args[args.indexOf('--environment') + 1];
if (project !== m.railway.projectId || environment !== m.railway.staging.environmentId) process.exit(8);
appendFileSync(process.env.CALLS_PATH, args[0] + '\\n');
if (args[0] === 'status') {
  console.log(JSON.stringify({ id: m.railway.projectId, environments: { edges: [{ node: {
    id: m.railway.staging.environmentId, name: m.railway.staging.environmentName,
  } }] } }));
} else if (args[0] === 'service' && args[1] === 'list') {
  console.log(JSON.stringify([...Object.values(m.services),
    { id: m.railway.databaseServiceId, name: 'scope-postgres' }]));
} else if (args[0] === 'variable' && args[1] === 'list') {
  console.log(JSON.stringify({ DATABASE_PUBLIC_URL: 'postgresql://smoke-db.example.test/scope' }));
} else if (args[0] === 'run') {
  const command = args.slice(args.indexOf('--') + 1);
  const result = spawnSync(command[0], command.slice(1), { stdio: 'inherit', env: {
    ...process.env, RAILWAY_PROJECT_ID: project, RAILWAY_ENVIRONMENT_ID: environment,
    RAILWAY_ENVIRONMENT_NAME: m.railway.staging.environmentName,
  } });
  process.exit(result.status ?? 1);
} else process.exit(9);
`, { mode: 0o755 });
  const seedBinary = join(root, 'seed');
  writeFileSync(seedBinary, `#!/usr/bin/env node
const { writeFileSync } = require('node:fs');
const assert = require('node:assert/strict');
const e = process.env;
assert.equal(e.SCOPE_SMOKE_SEED_PROJECT_ID, e.RAILWAY_PROJECT_ID);
assert.equal(e.SCOPE_SMOKE_SEED_ENVIRONMENT_ID, e.RAILWAY_ENVIRONMENT_ID);
assert.equal(e.SCOPE_SMOKE_SEED_ENVIRONMENT_NAME, e.RAILWAY_ENVIRONMENT_NAME);
assert.notEqual(e.RAILWAY_ENVIRONMENT_ID, e.SCOPE_PRODUCTION_ENVIRONMENT_ID);
assert.equal(e.SCOPE_ALLOW_STAGING_SMOKE_SEED, '1');
assert.equal(e.DATABASE_URL, 'postgresql://smoke-db.example.test/scope');
writeFileSync(e.SEED_ARGS_PATH, JSON.stringify(process.argv.slice(2)));
writeFileSync(e.SCOPE_SMOKE_SEED_EXCHANGE_TOKEN_PATH, 'scope_otc_test\\n', { mode: 0o600, flag: 'wx' });
`, { mode: 0o755 });
  const env = {
    ...process.env,
    PATH: `${join(root, 'bin')}:${process.env.PATH}`,
    RAILWAY_TOKEN: 'test-project-token',
    RAILWAY_API_TOKEN: '',
    SCOPE_DEPLOYMENT_MANIFEST: manifestPath,
    SCOPE_SMOKE_SEED_BINARY: seedBinary,
    SCOPE_SMOKE_SEED_EXCHANGE_TOKEN_PATH: join(root, 'credentials/exchange-token'),
    CALLS_PATH: join(root, 'calls'),
    SEED_ARGS_PATH: join(root, 'seed-args.json'),
  };
  return { root, env, manifestPath };
}

for (const [mode, args] of [['seed', []], ['grant', ['--grant-only']]]) {
  test(`${mode} uses the reviewed non-production identity and keeps the token private`, (t) => {
    const { env } = fixture(t);
    const result = spawnSync('bash', [seedScript, mode], { env, encoding: 'utf8' });
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(JSON.parse(readFileSync(env.SEED_ARGS_PATH)), args);
    assert.equal(statSync(env.SCOPE_SMOKE_SEED_EXCHANGE_TOKEN_PATH).mode & 0o777, 0o600);
    assert.doesNotMatch(result.stdout + result.stderr, /scope_otc_test|postgresql:/);
    assert.deepEqual(readFileSync(env.CALLS_PATH, 'utf8').trim().split('\n'), ['status', 'service', 'variable', 'run']);
  });
}

test('production aliases and account tokens cannot issue smoke credentials', (t) => {
  for (const invalid of ['production', 'account-token']) {
    const { env, manifestPath } = fixture(t);
    if (invalid === 'production') {
      const changed = structuredClone(manifest);
      changed.railway.staging.environmentId = changed.railway.environmentId;
      writeFileSync(manifestPath, JSON.stringify(changed));
    } else env.RAILWAY_API_TOKEN = 'test-account-token';
    const result = spawnSync('bash', [seedScript, 'grant'], { env, encoding: 'utf8' });
    assert.notEqual(result.status, 0);
    assert.equal(existsSync(env.SCOPE_SMOKE_SEED_EXCHANGE_TOKEN_PATH), false);
    if (existsSync(env.CALLS_PATH)) assert.doesNotMatch(readFileSync(env.CALLS_PATH, 'utf8'), /variable|run/);
  }
});

function workflowStep(name) {
  const block = workflow.split(`      - name: ${name}\n`)[1]?.split('\n      - name:')[0];
  assert.ok(block, `${name} is present`);
  return block;
}

function runStep(name, cwd, env) {
  const body = workflowStep(name).split('        run: |\n')[1];
  assert.ok(body, `${name} has an executable script`);
  const script = body.split('\n').map((line) => line.replace(/^          /, '')).join('\n');
  const result = spawnSync('bash', ['-euo', 'pipefail', '-c', script], { cwd, env, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
}

test('imported releases extract smoke tools and initialize credentials without backend archives', (t) => {
  const { root, env } = fixture(t);
  mkdirSync(join(root, 'artifacts/commands'), { recursive: true });
  for (const name of ['scope', 'scope-smoke-seed']) {
    writeFileSync(join(root, 'artifacts/commands', name), '#!/bin/sh\nexit 0\n', { mode: 0o755 });
  }
  const packed = spawnSync('tar', ['-czf', 'artifacts/staging-commands.tar.gz', '-C', 'artifacts/commands', '.'], { cwd: root });
  assert.equal(packed.status, 0);
  env.GITHUB_ENV = join(root, 'github-env');
  env.RUNNER_TEMP = root;
  runStep('Extract smoke tools', root, env);
  runStep('Initialize smoke credentials directory', root, env);
  const emitted = Object.fromEntries(readFileSync(env.GITHUB_ENV, 'utf8').trim().split('\n').map((line) => line.split('=')));
  assert.equal(statSync(join(root, 'candidate/cli/target/release/scope')).mode & 0o111, 0o111);
  assert.equal(statSync(join(root, 'candidate/target/release/scope-smoke-seed')).mode & 0o111, 0o111);
  assert.equal(statSync(emitted.SCOPE_GIT_SMOKE_DIR).mode & 0o777, 0o700);
  assert.equal(emitted.SCOPE_SMOKE_SEED_EXCHANGE_TOKEN_PATH, join(emitted.SCOPE_GIT_SMOKE_DIR, 'exchange-token'));
  for (const name of ['Build smoke tools', 'Upload staging commands', 'Extract smoke tools', 'Initialize smoke credentials directory']) {
    assert.doesNotMatch(workflowStep(name), /\n        if:/);
  }
  assert.match(workflowStep('Issue smoke login for imported release'), /staging-smoke-seed\.sh grant/);
});
