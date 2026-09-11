import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { APPLICATION_COMPONENTS, BACKEND_COMPONENTS, RAILWAY_COMPONENTS, backendSelected, deploymentComponent, loadComponentConfig } from './deployment-components.mjs';
import { artifactDeploymentInput } from './railway-artifact.mjs';

const sourceSha = 'a'.repeat(40);
const digest = `sha256:${'b'.repeat(64)}`;

test('every component has one runtime contract and a consistent backend selection', () => {
  assert.deepEqual(BACKEND_COMPONENTS.slice().sort(), ['api', 'cache', 'git-router', 'media-api', 'media-worker', 'run-worker']);
  assert.equal(backendSelected({}), false);
  for (const component of RAILWAY_COMPONENTS) {
    const definition = deploymentComponent(component);
    assert.equal(typeof definition.backend, 'boolean');
    assert.equal(typeof definition.verifyTransitionConfig, 'boolean');
    const config = loadComponentConfig(component);
    assert.equal(typeof config.deploy.healthcheckPath, 'string', component);
    assert.ok(Number.isInteger(config.deploy.healthcheckTimeout), component);
    assert.equal(backendSelected({ [component]: true }), BACKEND_COMPONENTS.includes(component));
    assert.equal(backendSelected({ [component]: false }), false);
    const plan = spawnSync(process.execPath, ['.github/scripts/plan-production-deployment.mjs', '--scope', component], {
      encoding: 'utf8', env: { ...process.env, GITHUB_OUTPUT: '', GITHUB_STEP_SUMMARY: '' },
    });
    assert.equal(plan.status, 0, plan.stderr);
    assert.ok(plan.stdout.includes(`backend_selected=${BACKEND_COMPONENTS.includes(component)}\n`));
    const artifact = artifactDeploymentInput(component, { image: `ghcr.io/test/image@${digest}` }, config);
    if (definition.artifact.kind !== 'web') {
      assert.match(definition.artifact.binary, /^scope-[a-z-]+$/);
      assert.equal(artifact.startCommand, `/app/bin/${definition.artifact.binary}`);
      if (config.deploy.startCommand) assert.equal(`exec .${artifact.startCommand.slice('/app'.length)}`, config.deploy.startCommand);
    }
  }
  assert.equal(APPLICATION_COMPONENTS.includes('cli-downloads'), false, 'CLI upload remains independent');
  assert.equal(deploymentComponent('media-worker').artifact.kind, 'external-image');
});

for (const [component, binary, dockerfile, installGit, suffix] of [
  ['api', 'scope-vcs', 'deploy/railway/prebuilt.Dockerfile', '1', 'api'],
  ['cache', 'scope-cache-service', 'deploy/railway/prebuilt.Dockerfile', '0', 'cache'],
  ['web', '', 'deploy/railway/web.Dockerfile', '0', 'web'],
]) {
  test(`artifact preparation and activation agree on ${component}`, t => {
    const root = mkdtempSync(join(tmpdir(), 'scope-component-preparation-'));
    t.after(() => rmSync(root, { recursive: true, force: true }));
    const bin = join(root, 'tools');
    const context = join(root, 'context');
    mkdirSync(bin);
    mkdirSync(join(context, 'bin'), { recursive: true });
    const maintenance = join(context, 'bin/scope-maintenance');
    writeFileSync(maintenance, 'maintenance fixture', { mode: 0o755 });
    if (binary) writeFileSync(join(context, 'bin', binary), 'binary fixture', { mode: 0o755 });
    else {
      mkdirSync(join(context, '.output/server'), { recursive: true });
      writeFileSync(join(context, '.output/server/index.mjs'), 'web fixture');
    }
    // Package-visibility behavior has its own transport tests. Stub only that
    // external boundary; exercise the actual metadata reader and release writer.
    writeFileSync(join(bin, 'node'), `#!/bin/sh
if [ "$1" = .github/scripts/railway-artifact.mjs ] && [ "$2" = verify-private-package ]; then exit 0; fi
exec "$TEST_NODE_BINARY" "$@"
`, { mode: 0o755 });
    writeFileSync(join(bin, 'docker'), `#!/usr/bin/env node
const fs = require('node:fs');
const args = process.argv.slice(2);
fs.appendFileSync(process.env.TEST_DOCKER_TRACE, JSON.stringify(args) + '\\n');
if (args[0] === 'buildx') fs.writeFileSync(args[args.indexOf('--metadata-file') + 1], JSON.stringify({'containerimage.digest': ${JSON.stringify(digest)}}));
if (args[0] === 'login') fs.readFileSync(0);
`, { mode: 0o755 });
    const releasePath = join(root, 'release.json');
    const trace = join(root, 'docker.jsonl');
    const result = spawnSync('bash', ['.github/scripts/prepare-railway-artifact.sh', component, context, releasePath], {
      encoding: 'utf8', timeout: 15_000,
      env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, TEST_NODE_BINARY: process.execPath,
        TEST_DOCKER_TRACE: trace, GITHUB_REPOSITORY: 'test/repo', GITHUB_TOKEN: 'fixture-token',
        SCOPE_DEPLOYMENT_SOURCE_SHA: sourceSha, SCOPE_DEPLOYMENT_MANIFEST: '.github/deployment-services.json',
        SCOPE_RAILWAY_REGISTRY_USERNAME: 'fixture-user', SCOPE_RAILWAY_REGISTRY_PASSWORD: 'fixture-password',
        SCOPE_MAINTENANCE_BINARY: maintenance,
      },
    });
    assert.equal(result.status, 0, result.stderr);
    const calls = readFileSync(trace, 'utf8').trim().split('\n').map(JSON.parse);
    const build = calls.find(args => args[0] === 'buildx');
    assert.equal(build[build.indexOf('--file') + 1], dockerfile);
    assert.ok(build.includes(`BINARY=${binary}`));
    assert.ok(build.includes(`INSTALL_GIT=${installGit}`));
    const release = JSON.parse(readFileSync(releasePath, 'utf8'));
    assert.equal(release.components[component].image, `ghcr.io/test/repo/railway-private-${suffix}@${digest}`);
    assert.equal(release.components[component].sourceSha, sourceSha);
    assert.equal(readFileSync(join(context, '.scope-deployment-sha'), 'utf8').trim(), sourceSha);
    assert.ok(calls.some(args => args[0] === 'manifest' && args.includes(release.components[component].image)));
  });
}
