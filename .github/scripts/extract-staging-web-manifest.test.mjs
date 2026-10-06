import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

import { copyWebManifest, stagingWebImage } from './extract-staging-web-manifest.mjs';
import { releaseImageRepository } from './railway-artifact.mjs';

const manifest = JSON.parse(readFileSync(new URL('../deployment-services.json', import.meta.url)));
const repository = 'scope-vcs/scope-vcs';
const sourceSha = 'a'.repeat(40);
const image = `${releaseImageRepository(manifest, repository, 'web')}@sha256:${'b'.repeat(64)}`;
const serviceId = manifest.services.web.id;
const environmentId = manifest.environments.staging.environmentId;

function fixture(webSelected = true) {
  const prepared = { schemaVersion: 1, sourceSha, components: webSelected ? {
    web: { image, sourceSha, serviceId },
  } : {} };
  const deployment = { id: 'web-deployment', status: 'SUCCESS', serviceId, environmentId,
    meta: { image, imageDigest: `sha256:${'b'.repeat(64)}` } };
  const status = { environments: { edges: [{ node: { id: environmentId,
    serviceInstances: { edges: [{ node: { serviceId, serviceName: 'web', numReplicas: 1,
      activeDeployments: [{ id: deployment.id, status: 'SUCCESS', deploymentStopped: false,
        instances: [{ status: 'RUNNING' }], meta: { serviceManifest: { deploy: { numReplicas: 1 } } } }],
      latestDeployment: null,
    } }] },
  } }] } };
  return { prepared, status, history: [deployment] };
}

test('extracts the exact selected web image and the trusted retained web image', () => {
  for (const selected of [true, false]) {
    const state = fixture(selected);
    assert.equal(stagingWebImage(state.prepared, manifest, repository, state.status, state.history), image);
  }
});

test('rejects missing, substituted, and unproven live web images', () => {
  for (const mutate of [
    state => { state.history = []; },
    state => { state.history[0].status = 'FAILED'; },
    state => { state.history[0].meta.image = null; },
    state => { state.history[0].meta.image = `ghcr.io/attacker/web@sha256:${'b'.repeat(64)}`; },
    state => { state.history[0].meta.image = image.replace('b'.repeat(64), 'c'.repeat(64)); },
    state => { state.history[0].environmentId = manifest.environments.production.environmentId; },
    state => { state.status.environments.edges[0].node.serviceInstances.edges[0].node.activeDeployments[0].id = 'other'; },
  ]) {
    const state = fixture();
    mutate(state);
    assert.throws(() => stagingWebImage(state.prepared, manifest, repository, state.status, state.history));
  }
  const retained = fixture(false);
  delete retained.history[0].meta.image;
  assert.throws(() => stagingWebImage(retained.prepared, manifest, repository, retained.status, retained.history), /allowlisted immutable digest/);
});

test('copies a manifest from a stopped container without executing the image', () => {
  const directory = mkdtempSync(join(tmpdir(), 'scope-web-manifest-test-'));
  const destination = join(directory, 'ssr.mjs');
  const commands = [];
  try {
    copyWebManifest(image, destination, {
      registryUsername: 'reader', registryPassword: 'secret',
      execute: (_command, args) => {
        commands.push(args);
        if (args[0] === 'cp') writeFileSync(args[2], 'export const manifest = {};\n');
      },
    });
    assert.match(readFileSync(destination, 'utf8'), /manifest/);
    assert.deepEqual(commands.map(args => args[0]), ['login', 'pull', 'create', 'cp', 'container']);
    assert(commands.some(args => args[0] === 'create' && args.includes('--network') && args.includes('none')));
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

function copyWithLoginFailures(failures) {
  const directory = mkdtempSync(join(tmpdir(), 'scope-web-manifest-test-'));
  const commands = [];
  const pauses = [];
  try {
    let error;
    try {
      copyWebManifest(image, join(directory, 'ssr.mjs'), {
        registryUsername: 'reader', registryPassword: 'secret',
        pause: milliseconds => pauses.push(milliseconds),
        execute: (_command, args) => {
          commands.push(args[0]);
          if (args[0] === 'login' && commands.length <= failures) {
            throw new Error('Get "https://ghcr.io/v2/": context deadline exceeded');
          }
          if (args[0] === 'cp') writeFileSync(args[2], 'export const manifest = {};\n');
        },
      });
    } catch (caught) {
      error = caught;
    }
    return { commands, pauses, error };
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

test('retries a transient registry login with backoff before pulling', () => {
  const { commands, pauses, error } = copyWithLoginFailures(2);
  assert.ifError(error);
  assert.deepEqual(commands, ['login', 'login', 'login', 'pull', 'create', 'cp', 'container']);
  assert.deepEqual(pauses, [5_000, 10_000]);
});

test('fails with the registry error after three login attempts without pulling', () => {
  const { commands, pauses, error } = copyWithLoginFailures(3);
  assert.match(error.message, /Registry login failed after 3 attempts: ghcr\.io\n.*context deadline exceeded/);
  assert.deepEqual(commands, ['login', 'login', 'login', 'container']);
  assert.deepEqual(pauses, [5_000, 10_000]);
});

test('fails if the image does not contain the compiled manifest', () => {
  const directory = mkdtempSync(join(tmpdir(), 'scope-web-manifest-test-'));
  try {
    assert.throws(() => copyWebManifest(image, join(directory, 'ssr.mjs'), {
      execute: () => {},
    }), /lacks a compiled server manifest/);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
