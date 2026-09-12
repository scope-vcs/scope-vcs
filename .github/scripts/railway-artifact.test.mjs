import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import assert from 'node:assert/strict';
import { activateArtifact, artifactDeploymentInput, assertActivatedArtifact, configureStagingRegistry, releaseImageRepository, validateMaintenanceArtifact, validatePreparedRelease, verifyPrivateReleasePackage } from './railway-artifact.mjs';

const sourceSha = 'a'.repeat(40);
const image = `ghcr.io/owner/repo/railway-private-api@sha256:${'b'.repeat(64)}`;
const release = () => ({ schemaVersion: 1, sourceSha, components: { api: { sourceSha, image, serviceId: 'api-id' } } });
const config = { deploy: { healthcheckPath: '/readyz', healthcheckTimeout: 60, overlapSeconds: 30, drainingSeconds: 30, numReplicas: 1 } };

test('default Railway transport retries a failed source read without repeating either mutation', (t) => {
  const directory = mkdtempSync(join(tmpdir(), 'scope-artifact-read-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const events = join(directory, 'events');
  writeFileSync(join(directory, 'railway'), `#!/usr/bin/env node
    const fs = require('node:fs');
    const query = process.argv[3];
    const variables = JSON.parse(fs.readFileSync(0, 'utf8'));
    const events = ${JSON.stringify(events)};
    const previous = fs.existsSync(events) ? fs.readFileSync(events, 'utf8') : '';
    const operation = query.match(/^(?:query|mutation) (\\w+)/)[1];
    fs.appendFileSync(events, operation + '\\n');
    if (operation === 'SelectedSource' && !previous.includes('SelectedSource')) {
      console.log('partial failed response');
      process.exit(1);
    }
    const image = ${JSON.stringify(image)};
    const data = operation === 'ActivateSource' ? { serviceInstanceUpdate: true }
      : operation === 'SelectedSource' ? { serviceInstance: { source: { image } } }
      : operation === 'SelectedConfig' ? { environment: { config: { services: { 'api-id': { source: { image } } } } } }
      : { serviceInstanceDeployV2: 'new-api' };
    console.log(JSON.stringify({ data }));
  `, { mode: 0o755 });
  const script = `
    import { activateArtifact } from ${JSON.stringify(new URL('./railway-artifact.mjs', import.meta.url).href)};
    console.log(JSON.stringify(activateArtifact(${JSON.stringify(release())}, 'api', 'staging-id', { config: ${JSON.stringify(config)} })));
  `;
  const result = spawnSync(process.execPath, ['--input-type=module', '-e', script], {
    env: { ...process.env, PATH: `${directory}:${process.env.PATH}` },
    encoding: 'utf8', timeout: 10000,
  });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(JSON.parse(result.stdout).deploymentId, 'new-api');
  assert.deepEqual(readFileSync(events, 'utf8').trim().split('\n'), [
    'ActivateSource', 'SelectedSource', 'SelectedSource', 'SelectedConfig', 'ActivateImage',
  ]);
});

test('release rejects mutable tags, missing components, wrong revision and wrong service before activation', () => {
  assert.throws(() => validatePreparedRelease(release(), { sourceSha: 'c'.repeat(40) }), /revision/);
  assert.throws(() => validatePreparedRelease(release(), { components: ['run-worker'] }), /missing run-worker/);
  assert.throws(() => validatePreparedRelease(release(), { services: { api: { id: 'wrong' } } }), /wrong service/);
  const mutable = release(); mutable.components.api.image = 'ghcr.io/owner/repo/railway-private-api:latest';
  assert.throws(() => validatePreparedRelease(mutable), /immutable/);
  const wrongSource = release(); wrongSource.components.api.sourceSha = 'c'.repeat(40);
  assert.throws(() => validatePreparedRelease(wrongSource), /source revision/);
});

test('image activation applies readiness and drains but preserves target replica topology', () => {
  const input = artifactDeploymentInput('api', release().components.api, config);
  assert.equal(input.healthcheckPath, '/readyz');
  assert.equal(input.overlapSeconds, 30);
  assert.equal(input.drainingSeconds, 30);
  assert.equal(input.startCommand, '/app/bin/scope-vcs');
  assert.deepEqual(input.source, { image });
  assert.equal(Object.hasOwn(input, 'numReplicas'), false);
  assert.equal(Object.hasOwn(input, 'registryCredentials'), false);
  assert.throws(() => artifactDeploymentInput('api', release().components.api, config, { registryCredentials: { username: 'user' } }), /Both/);
});

test('activation uses environment-scoped update, verifies selected digest, and returns exact deployment ID', () => {
  const calls = [];
  const result = activateArtifact(release(), 'api', 'staging-id', { config, railway(query, variables) {
    calls.push({ query, variables });
    if (query.startsWith('mutation ActivateSource')) return { data: { serviceInstanceUpdate: true } };
    if (query.startsWith('query SelectedSource')) return { data: { serviceInstance: { source: { image, repo: null } } } };
    if (query.startsWith('query SelectedConfig')) return { data: { environment: { config: { services: { 'api-id': { source: { image } } } } } } };
    return { data: { serviceInstanceDeployV2: 'exact-deployment' } };
  } });
  assert.equal(result.deploymentId, 'exact-deployment');
  assert.equal(calls.length, 4);
  assert.ok(calls.every(({ variables }) => variables.environmentId === 'staging-id'));
  assert.ok(calls[0].query.includes('serviceInstanceUpdate'));
});

test('failed source verification prevents creating a deployment', () => {
  let calls = 0;
  assert.throws(() => activateArtifact(release(), 'api', 'staging-id', { config, railway() {
    calls += 1;
    return calls === 1 ? { data: { serviceInstanceUpdate: true } } : { data: { serviceInstance: { source: { image: 'wrong' } } } };
  } }), /read-back/);
  assert.equal(calls, 2);
});

test('canonical config mismatch prevents activation even when service instance read-back looks correct', () => {
  let calls = 0;
  assert.throws(() => activateArtifact(release(), 'api', 'staging-id', { config, railway(query) {
    calls += 1;
    if (query.startsWith('mutation ActivateSource')) return { data: { serviceInstanceUpdate: true } };
    if (query.startsWith('query SelectedSource')) return { data: { serviceInstance: { source: { image } } } };
    return { data: { environment: { config: { services: { 'api-id': { source: { repo: 'old/repo' } } } } } } };
  } }), /canonical environment/);
  assert.equal(calls, 3);
});

test('maintenance recovery metadata is validated independently of image identity', () => {
  const manifest = { ...release(), maintenanceSha256: 'c'.repeat(64), preparationRunId: '12345' };
  assert.equal(validatePreparedRelease(manifest), manifest);
  assert.throws(() => validatePreparedRelease({ ...manifest, maintenanceSha256: 'bad' }), /maintenance binary/);
  assert.throws(() => validatePreparedRelease({ ...manifest, preparationRunId: '../run' }), /preparation run/);
});


test('successful deployment must carry exact immutable artifact evidence', () => {
  const deployment = { id: 'exact-id', status: 'SUCCESS', serviceId: 'api-id', meta: { image, imageDigest: image.split('@')[1] } };
  assert.equal(assertActivatedArtifact(release(), 'api', deployment, { deploymentId: 'exact-id' }), deployment);
  assert.throws(() => assertActivatedArtifact(release(), 'api', deployment, { deploymentId: 'newer-id' }), /exact activated/);
  assert.throws(() => assertActivatedArtifact(release(), 'api', { ...deployment, status: 'BUILDING' }, { deploymentId: 'exact-id' }), /SUCCESS/);
  assert.throws(() => assertActivatedArtifact(release(), 'api', { ...deployment, meta: {} }, { deploymentId: 'exact-id' }), /no immutable/);
  assert.throws(() => assertActivatedArtifact(release(), 'api', { ...deployment, meta: { imageDigest: `sha256:${'c'.repeat(64)}` } }, { deploymentId: 'exact-id' }), /digest differs/);
  assert.throws(() => assertActivatedArtifact(release(), 'api', { ...deployment, meta: { ...deployment.meta, image: 'old:image' } }, { deploymentId: 'exact-id' }), /image differs/);
  assert.doesNotThrow(() => assertActivatedArtifact(release(), 'api', { ...deployment, meta: { serviceManifest: { source: { image } } } }, { deploymentId: 'exact-id' }));
});


test('maintenance binary is bound to the original prepared release', () => {
  const binary = Buffer.from('original maintenance binary');
  const hash = createHash('sha256').update(binary).digest('hex');
  const manifest = { ...release(), maintenanceSha256: hash };
  assert.equal(validateMaintenanceArtifact(manifest, binary), hash);
  assert.throws(() => validateMaintenanceArtifact(release(), binary), /missing its maintenance/);
  assert.throws(() => validateMaintenanceArtifact(manifest, Buffer.alloc(0)), /missing or empty/);
  assert.throws(() => validateMaintenanceArtifact(manifest, Buffer.from('rebuilt binary')), /does not match/);
});


test('checked-in Railway string durations become GraphQL integers', () => {
  for (const [component, directory] of [['api', 'api'], ['run-worker', 'worker'], ['cache', 'cache-service'], ['git-router', 'repo-router'], ['media-api', 'media-service'], ['web', 'web']]) {
    const actualConfig = JSON.parse(readFileSync(new URL(`../../${directory}/railway.json`, import.meta.url), 'utf8'));
    const input = artifactDeploymentInput(component, release().components.api, actualConfig);
    assert.equal(input.overlapSeconds, 30);
    assert.equal(input.drainingSeconds, 30);
  }
  for (const value of [null, true, '', '30seconds', '-1', 1.5, 2147483648]) {
    assert.throws(() => artifactDeploymentInput('api', release().components.api, { deploy: { ...config.deploy, overlapSeconds: value } }), /GraphQL Int/);
  }
});


test('trusted staging registry configuration touches only fixed environment credentials', () => {
  const manifest = JSON.parse(readFileSync(new URL('../deployment-services.json', import.meta.url), 'utf8'));
  const calls = [];
  const credentials = { username: 'registry-user', password: 'private-pull-token' };
  const result = configureStagingRegistry(manifest, credentials, (query, variables) => {
    calls.push({ query, variables });
    return { data: { serviceInstanceUpdate: true } };
  });
  assert.deepEqual(result, { configured: true, serviceCount: 7 });
  assert.equal(calls.length, 7);
  assert.deepEqual(calls.map(({ variables }) => variables.serviceId), [manifest.services.cache.id, manifest.services['run-worker'].id, manifest.services['git-router'].id, manifest.services['media-api'].id, manifest.services['media-worker'].id, manifest.services.api.id, manifest.services.web.id]);
  for (const { query, variables } of calls) {
    assert.equal(variables.environmentId, manifest.environments.staging.environmentId);
    assert.deepEqual(variables.input, { registryCredentials: credentials });
    assert.ok(query.includes('serviceInstanceUpdate'));
    assert.ok(!query.includes('Deploy'));
  }
  assert.deepEqual(configureStagingRegistry(manifest, {}, () => assert.fail('empty credentials should not call Railway')), { configured: false });
  assert.throws(() => configureStagingRegistry(manifest, { username: 'partial' }), /Both/);
  assert.throws(() => configureStagingRegistry({ ...manifest, environments: { ...manifest.environments, staging: { ...manifest.environments.staging, environmentId: manifest.environments.production.environmentId } } }, credentials), /distinct/);
  assert.throws(() => configureStagingRegistry({ ...manifest, services: { ...manifest.services, api: {} } }, credentials), /api service/);
});


test('release package naming has one validated manifest owner', () => {
  const manifest = JSON.parse(readFileSync(new URL('../deployment-services.json', import.meta.url), 'utf8'));
  assert.equal(manifest.railway.releaseImagePrefix, 'railway-private');
  assert.equal(releaseImageRepository(manifest, 'Scope-VCS/Scope-VCS', 'api'), 'ghcr.io/scope-vcs/scope-vcs/railway-private-api');
  for (const [component, imageSuffix, binary] of [
    ['run-worker', 'worker', 'scope-worker'],
    ['git-router', 'router', 'scope-repo-router'],
    ['media-api', 'media', 'scope-media-service'],
  ]) {
    assert.equal(releaseImageRepository(manifest, 'Scope-VCS/Scope-VCS', component), `ghcr.io/scope-vcs/scope-vcs/railway-private-${imageSuffix}`);
    assert.equal(artifactDeploymentInput(component, release().components.api, config).startCommand, `/app/bin/${binary}`);
  }
  assert.throws(() => releaseImageRepository(manifest, 'Scope-VCS/Scope-VCS', 'worker'), /Unknown release component/);
  assert.equal(releaseImageRepository({ railway: { releaseImagePrefix: 'another-release-set' } }, 'owner/repo', 'web'), 'ghcr.io/owner/repo/another-release-set-web');
  for (const prefix of [undefined, '', '../escape', 'registry/package', 'MixedCase']) {
    assert.throws(() => releaseImageRepository({ railway: { releaseImagePrefix: prefix } }, 'owner/repo', 'api'), /releaseImagePrefix/);
  }
  assert.throws(() => releaseImageRepository(manifest, 'owner/repo/escape', 'api'), /OWNER\/REPOSITORY/);
  assert.throws(() => releaseImageRepository(manifest, 'owner/repo', '../api'), /Unknown release component/);
});


for (const [accountType, namespace] of [['Organization', 'orgs'], ['User', 'users']]) {
  test(`private package verification uses the exact encoded package for a ${accountType}`, async () => {
    const manifest = { railway: { releaseImagePrefix: 'railway-private' } };
    const calls = [];
    const result = await verifyPrivateReleasePackage(manifest, 'Owner/Repo', 'api', {
      token: 'publishing-token', fetchImpl: async (url, options) => {
        calls.push({ url, options });
        const body = calls.length === 1 ? { login: 'Owner', type: accountType } : { name: 'repo/railway-private-api', package_type: 'container', owner: { login: 'owner' }, visibility: 'private' };
        return new Response(JSON.stringify(body));
      },
    });
    assert.deepEqual(calls.map(({ url }) => url), ['https://api.github.com/users/owner', `https://api.github.com/${namespace}/owner/packages/container/repo%2Frailway-private-api`]);
    assert.ok(calls.every(({ options }) => options.headers.Authorization === 'Bearer publishing-token' && options.redirect === 'error'));
    assert.deepEqual(result, { imageRepository: 'ghcr.io/owner/repo/railway-private-api', visibility: 'private' });
  });
}

test('package verification rejects public, internal, mismatched and unreadable package metadata', async () => {
  const manifest = { railway: { releaseImagePrefix: 'railway-private' } };
  for (const bad of [{ visibility: 'public' }, { visibility: 'internal' }, { name: 'repo/other' }, { package_type: 'npm' }, { owner: { login: 'someone-else' } }, { status: 403 }]) {
    await assert.rejects(verifyPrivateReleasePackage(manifest, 'owner/repo', 'api', {
      token: 'publishing-token', fetchImpl: async (url) => {
        if (url.endsWith('/users/owner')) return new Response(JSON.stringify({ login: 'owner', type: 'Organization' }));
        if (bad.status) return new Response('do not include response secrets', { status: bad.status });
        return new Response(JSON.stringify({ name: 'repo/railway-private-api', package_type: 'container', owner: { login: 'owner' }, visibility: 'private', ...bad }));
      },
    }), /must be private|different release package|HTTP 403/);
  }
  await assert.rejects(verifyPrivateReleasePackage(manifest, 'owner/repo', 'api'), /GITHUB_TOKEN/);
});

test('registry configuration retries the same service credentials without deploying', () => {
  const manifest = JSON.parse(readFileSync(new URL('../deployment-services.json', import.meta.url), 'utf8'));
  const calls = [];
  const credentials = { username: 'registry-user', password: 'private-pull-token' };
  configureStagingRegistry(manifest, credentials, (query, variables) => {
    calls.push({ query, variables });
    if (calls.length === 1) throw new Error('HTTP 500 with SECRET provider body');
    if (calls.length === 2) return { errors: [{ message: 'SECRET' }], data: { serviceInstanceUpdate: true } };
    return { data: { serviceInstanceUpdate: true } };
  });
  assert.equal(calls.length, 9);
  assert.deepEqual(calls[0], calls[1]);
  assert.deepEqual(calls[1], calls[2]);
  assert.ok(calls.every(({ query }) => !query.includes('Deploy')));
});
