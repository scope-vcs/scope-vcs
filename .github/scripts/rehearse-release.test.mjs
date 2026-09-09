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

test('both staging entry points prepare commands and fixtures before running smoke', () => {
  assert.doesNotMatch(job('prepare').split('    steps:')[0], /inputs\.prepared_run_id/);
  assert.match(job('prove').split('    steps:')[0], /needs\.prepare\.result == 'success'/);
  for (const name of [
    'Close staging writers', 'Download prepared candidate artifacts', 'Extract candidate commands',
    'Migrate, seed, and deploy staging services', 'Deploy staging web and record evidence',
  ]) {
    assert.doesNotMatch(step('prove', name), /\n        if:/, `${name} must run for imported images too`);
  }
  assert.doesNotMatch(step('cleanup', 'Keep staging writers fenced after unsuccessful proof'), /inputs\.prepared_run_id/);
});

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
    run(script('prove', 'Migrate, seed, and deploy staging services'), candidate);
    const smokeDir = readFileSync(env.GITHUB_ENV, 'utf8').trim().split('=')[1];
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
