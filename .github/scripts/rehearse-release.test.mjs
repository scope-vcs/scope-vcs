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

test('candidate preparation precedes the job that can stop staging writers', () => {
  const workflow = readFileSync(new URL('../workflows/scope-railway-staging.yml', import.meta.url), 'utf8');
  const proof = workflow.slice(workflow.indexOf('\n  prove:'));
  assert.match(proof, /needs: \[prepare, prepare-images, media-worker-image\]/);
  assert(proof.indexOf('Close staging writers') < proof.indexOf('Checkout exact candidate revision'));
  assert.doesNotMatch(proof, /cargo build/);
  assert.match(proof, /ref: \$\{\{ github\.sha \}\}\n\s+persist-credentials: false/);
  assert.match(proof, /working-directory: candidate[\s\S]+rehearse-release\.mjs/);
});

const workflow = readFileSync(new URL('../workflows/scope-railway-staging.yml', import.meta.url), 'utf8');
const job = (name) => workflow.split(`\n  ${name}:\n`)[1].split(/\n  [\w-]+:\n/)[0];
const step = (jobName, name) => job(jobName).split(`      - name: ${name}\n`)[1].split('\n      - name:')[0];
const script = (jobName, name) => step(jobName, name).split('        run: |\n')[1]
  .split('\n').map((line) => line.replace(/^          /, '')).join('\n');

test('both staging entry points prepare commands while only complete releases reset fixtures', () => {
  assert.doesNotMatch(job('prepare').split('    steps:')[0], /inputs\.prepared_run_id/);
  assert.match(job('prove').split('    steps:')[0], /needs\.prepare\.result == 'success'/);
  for (const name of [
    'Download prepared candidate artifacts', 'Extract candidate commands',
    'Initialize smoke credentials directory',
  ]) {
    assert.doesNotMatch(step('prove', name), /\n        if:/, `${name} must run for imported images too`);
  }
  for (const name of ['Close staging writers', 'Migrate, seed, and deploy staging services', 'Deploy staging web and record evidence']) {
    assert.match(step('prove', name), /if: steps\.release\.outputs\.initialize_fixtures == 'true'/);
  }
  assert.match(step('prove', 'Issue smoke login for partial release'), /if: steps\.release\.outputs\.initialize_fixtures != 'true'/);
  assert.match(step('cleanup', 'Keep staging writers fenced after unsuccessful proof'), /needs\.prove\.outputs\.writers_fenced == 'true'/);
  assert.match(job('prove'), /writers_fenced: \$\{\{ steps\.fence\.outcome == 'success' \}\}/);
  assert(job('prove').indexOf('Export release paths') < job('prove').indexOf('Close staging writers'));
});

for (const [name, components, initialize] of [
  ['full', ['api', 'worker', 'cache', 'router', 'media', 'mediaWorker', 'web'], 'true'],
  ['web-only', ['web'], 'false'],
  ['backend-only', ['api', 'worker', 'cache', 'router', 'media', 'mediaWorker'], 'false'],
]) {
  test(`${name} manifest selects fixture initialization before fencing writers`, (t) => {
    const root = mkdtempSync(join(tmpdir(), 'scope-release-paths-'));
    t.after(() => rmSync(root, { recursive: true, force: true }));
    mkdirSync(join(root, 'artifacts'));
    const image = `ghcr.io/scope-vcs/scope-media-worker@sha256:${'a'.repeat(64)}`;
    writeFileSync(join(root, 'artifacts/prepared-release.json'), JSON.stringify({
      components: Object.fromEntries(components.map((component) => [component, { image }])),
    }));
    const env = { ...process.env, GITHUB_WORKSPACE: root,
      GITHUB_OUTPUT: join(root, 'outputs'), GITHUB_ENV: join(root, 'env') };
    execFileSync('bash', ['-euo', 'pipefail', '-c', script('prove', 'Export release paths')], { cwd: root, env });
    assert.equal(readFileSync(env.GITHUB_OUTPUT, 'utf8').trim(), `initialize_fixtures=${initialize}`);
    const exported = Object.fromEntries(readFileSync(env.GITHUB_ENV, 'utf8').trim().split('\n').map((line) => {
      const separator = line.indexOf('=');
      return [line.slice(0, separator), line.slice(separator + 1)];
    }));
    assert.equal(exported.SCOPE_PREPARED_RELEASE_PATH, join(root, 'artifacts/prepared-release.json'));
    assert.equal(exported.SCOPE_DEPLOYMENT_MANIFEST, join(root, '.github/deployment-services.json'));
    assert.equal(exported.SCOPE_MEDIA_WORKER_IMAGE, components.includes('mediaWorker') ? image : '');
  });
}

for (const imported of [false, true]) {
  test(`${imported ? 'imported' : 'new'} images provide executable commands, private credentials, and a deployment receipt`, (t) => {
    const root = mkdtempSync(join(tmpdir(), 'scope-staging-setup-'));
    t.after(() => rmSync(root, { recursive: true, force: true }));
    const bin = join(root, 'bin');
    mkdirSync(bin);
    mkdirSync(join(root, 'legal'));
    for (const name of ['LICENSE', 'NOTICE', 'legal/third-party-rust.txt']) writeFileSync(join(root, name), 'fixture');
    // Stub compilation, but execute the workflow's real packaging and extraction.
    writeFileSync(join(bin, 'cargo'), `#!/usr/bin/env node
const fs = require('node:fs');
const args = process.argv.slice(2);
fs.appendFileSync('builds.jsonl', JSON.stringify(args) + '\\n');
const bins = args.includes('--bin') ? [args[args.indexOf('--bin') + 1]] :
  ['scope-cache-service', 'worker', 'scope-repo-router', 'scope-media-service'];
const dir = args.includes('--manifest-path') ? 'cli/target/release' : 'target/release';
fs.mkdirSync(dir, { recursive: true });
for (const name of bins) fs.writeFileSync(dir + '/' + name, '#!/bin/sh\\nexit 0\\n', { mode: 0o755 });
`, { mode: 0o755 });
    const env = {
      ...process.env, PATH: `${bin}:${process.env.PATH}`, USE_PREPARED_IMAGES: imported ? '1' : '0',
      RUNNER_TEMP: root, GITHUB_ENV: join(root, 'github-env'),
      SCOPE_STAGING_EVIDENCE_PATH: join(root, 'staging-deployments.json'),
    };
    const run = (body, cwd = root) => execFileSync('bash', ['-euo', 'pipefail', '-c', body], { cwd, env, stdio: 'pipe' });
    run(script('prepare', 'Build deployment and smoke binaries'));
    const builds = readFileSync(join(root, 'builds.jsonl'), 'utf8').trim().split('\n').map(JSON.parse);
    assert.equal(builds.some((args) => args.includes('scope-cache-service')), !imported);
    assert.equal(builds.some((args) => args.includes('--bin') && args[args.indexOf('--bin') + 1] === 'api'), !imported);
    run(script('prove', 'Extract candidate commands'));
    for (const path of ['target/release/scope-maintenance', 'target/release/scope-smoke-seed', 'cli/target/release/scope']) {
      accessSync(join(root, 'candidate', path), constants.X_OK);
    }
    const candidate = join(root, 'candidate');
    mkdirSync(join(candidate, '.github/scripts'), { recursive: true });
    writeFileSync(join(candidate, '.github/scripts/deploy-staging-railway.sh'), `#!/usr/bin/env bash
set -euo pipefail
if [[ "$1" == prepare ]]; then
  umask 077
  printf 'fixture-exchange' > "$SCOPE_SMOKE_SEED_EXCHANGE_TOKEN_PATH"
else
  printf '{"deployments":[{"status":"SUCCESS"}]}' > "$SCOPE_STAGING_EVIDENCE_PATH"
fi
`);
    run(script('prove', 'Initialize smoke credentials directory'), candidate);
    Object.assign(env, Object.fromEntries(readFileSync(env.GITHUB_ENV, 'utf8').trim().split('\n').map((line) => line.split('='))));
    run(script('prove', 'Migrate, seed, and deploy staging services'), candidate);
    const smokeDir = env.SCOPE_GIT_SMOKE_DIR;
    assert.equal(statSync(smokeDir).mode & 0o777, 0o700);
    assert.equal(statSync(join(smokeDir, 'exchange-token')).mode & 0o777, 0o600);
    assert.equal(readFileSync(join(smokeDir, 'exchange-token'), 'utf8'), 'fixture-exchange');
    run(script('prove', 'Deploy staging web and record evidence'), candidate);
    assert.equal(JSON.parse(readFileSync(env.SCOPE_STAGING_EVIDENCE_PATH)).deployments[0].status, 'SUCCESS');
    env.SCOPE_GIT_SMOKE_DIR = smokeDir;
    run(script('prove', 'Remove staging Git smoke credentials'));
    assert.throws(() => accessSync(smokeDir), /ENOENT/);
  });
}
