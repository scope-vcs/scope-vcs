import assert from 'node:assert/strict'
import { execFileSync, spawnSync } from 'node:child_process'
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { verifyStagingTarget, verifyStagingTopology } from './verify-staging-target.mjs'

function fixture() {
  const manifest = {
    railway: {
      databaseServiceId: 'database',
      environmentId: 'production',
      projectId: 'project',
      staging: {
        apiReplicas: 3,
        apiDomain: 'api-staging.example.test',
        cacheDomain: 'cache-staging.example.test',
        environmentId: 'staging',
        environmentName: 'staging',
        routerDomain: 'router-staging.example.test',
        routerReplicas: 1,
        routerServiceId: 'router',
        routerServiceName: 'scope-repo-router',
        webDomain: 'web-staging.example.test',
      },
    },
    services: {
      api: { id: 'api', name: 'scope-api' },
      cache: { id: 'cache', name: 'scope-cache-service' },
      media: { id: 'media', name: 'scope-media' },
      mediaWorker: { id: 'media-worker', name: 'scope-media-worker' },
      web: { id: 'web', name: 'scope-web' },
      worker: { id: 'worker', name: 'scope-worker' },
    },
    mediaResources: {
      bucket: { id: 'media-bucket' },
      staging: { gatewayDomain: 'media-staging.example.test' },
    },
  }
  const services = [
    { id: 'database', name: 'scope-postgres' },
    { id: 'cache', name: 'scope-cache-service', status: 'SUCCESS', replicas: healthyReplicas(1) },
    { id: 'worker', name: 'scope-worker', status: 'SUCCESS', replicas: healthyReplicas(1) },
    { id: 'api', name: 'scope-api', status: 'SUCCESS', replicas: healthyReplicas(3) },
    { id: 'router', name: 'scope-repo-router', status: 'SUCCESS', replicas: healthyReplicas(1) },
    { id: 'media', name: 'scope-media', status: 'SUCCESS', replicas: healthyReplicas(1) },
    { id: 'media-worker', name: 'scope-media-worker', status: 'SUCCESS', replicas: healthyReplicas(1) },
    { id: 'web', name: 'scope-web' },
  ]
  const status = {
    environments: { edges: [{ node: { id: 'staging', name: 'staging' } }] },
    id: 'project',
  }
  return { manifest, services, status }
}

function healthyReplicas(count) {
  return { configured: count, running: count, crashed: 0 }
}

test('staging deploys and records the candidate checkout when the workflow revision differs', (t) => {
  const root = mkdtempSync(join(tmpdir(), 'scope-staging-revision-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const scripts = join(root, '.github/scripts')
  const bin = join(root, 'bin')
  mkdirSync(scripts, { recursive: true })
  mkdirSync(bin)
  execFileSync('git', ['init', '--quiet', root])
  execFileSync('git', ['-c', 'user.name=Scope Test', '-c', 'user.email=scope@example.test',
    '-c', 'commit.gpgsign=false', 'commit', '--quiet', '--allow-empty', '-m', 'candidate'], { cwd: root })
  const candidate = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim()
  const input = fixture()
  writeFileSync(join(root, '.github/deployment-services.json'), JSON.stringify(input.manifest))
  copyFileSync(new URL('./verify-staging-target.mjs', import.meta.url), join(scripts, 'verify-staging-target.mjs'))
  copyFileSync(new URL('./deploy-staging-railway.sh', import.meta.url), join(scripts, 'deploy-staging-railway.sh'))
  writeFileSync(join(scripts, 'railway-read.mjs'), `
    const input = ${JSON.stringify(input)};
    const args = process.argv.slice(2);
    const command = args.slice(0, 2).join(' ');
    const service = args[args.indexOf('--service') + 1];
    if (args[0] === 'status') console.log(JSON.stringify(input.status));
    else if (command === 'service list') console.log(JSON.stringify(input.services));
    else if (command === 'variable list') console.log(JSON.stringify(service === 'api' ? {
      SCOPE_CACHE_URL: 'https://' + input.manifest.railway.staging.cacheDomain,
      SCOPE_GIT_PUBLIC_URL: 'https://' + input.manifest.railway.staging.routerDomain,
    } : { SCOPE_REPO_ROUTER_BACKEND: 'scope-api.railway.internal:8080', SCOPE_REPO_ROUTER_READ_REPLICAS: '3' }));
    else if (command === 'deployment list') console.log(JSON.stringify([{ id: service + '-deployment', status: 'SUCCESS' }]));
    else throw new Error('Unexpected provider read: ' + args.join(' '));
  `)
  writeFileSync(join(bin, 'railway'), '#!/bin/sh\ntest "$1 $2" = "variable set"\n', { mode: 0o755 })
  writeFileSync(join(scripts, 'deploy-railway.sh'), `#!/bin/sh
    node -e 'require("node:fs").writeFileSync("deployment.json", JSON.stringify({ source: process.env.SCOPE_DEPLOYMENT_SOURCE_SHA, message: process.env.RAILWAY_DEPLOY_MESSAGE }))'
  `)
  const result = spawnSync('bash', [join(scripts, 'deploy-staging-railway.sh'), 'finish', 'web'], {
    cwd: root,
    encoding: 'utf8',
    timeout: 10_000,
    env: {
      ...process.env,
      PATH: `${bin}:${process.env.PATH}`,
      GITHUB_SHA: 'a'.repeat(40),
      RAILWAY_TOKEN: 'test-token',
      RAILWAY_API_TOKEN: '',
      SCOPE_DEPLOYMENT_SOURCE_SHA: '',
      SCOPE_DEPLOYMENT_MANIFEST: join(root, '.github/deployment-services.json'),
      SCOPE_STAGING_EVIDENCE_PATH: join(root, 'evidence.json'),
      SCOPE_MEDIA_WORKER_IMAGE: `ghcr.io/scope-vcs/scope-media-worker@sha256:${'b'.repeat(64)}`,
    },
  })
  assert.ifError(result.error)
  assert.equal(result.status, 0, result.stderr)
  assert.deepEqual(JSON.parse(readFileSync(join(root, 'deployment.json'), 'utf8')), {
    source: candidate,
    message: `Staging ${candidate}`,
  })
  assert.equal(JSON.parse(readFileSync(join(root, 'evidence.json'), 'utf8')).commit, candidate)
})

test('accepts the reviewed staging target', () => {
  assert.deepEqual(verifyStagingTarget(fixture()), {
    apiReplicas: 3,
    productionEnvironmentId: 'production',
    projectId: 'project',
    routerReplicas: 1,
    stagingEnvironmentId: 'staging',
    stagingEnvironmentName: 'staging',
  })
})

test('rejects production as staging', () => {
  const input = fixture()
  input.manifest.railway.staging.environmentId = 'production'
  assert.throws(() => verifyStagingTarget(input), /differ from production/)
})

test('rejects a different project, environment, or service', () => {
  for (const mutate of [
    (input) => { input.status.id = 'wrong' },
    (input) => { input.status.environments.edges[0].node.id = 'wrong' },
    (input) => { input.services = input.services.filter(({ id }) => id !== 'api') },
    (input) => { input.services = input.services.filter(({ id }) => id !== 'router') },
  ]) {
    const input = fixture()
    mutate(input)
    assert.throws(() => verifyStagingTarget(input))
  }
})

test('rejects invalid staging replica counts', () => {
  for (const [key, value] of [['apiReplicas', 0], ['routerReplicas', 1.5]]) {
    const input = fixture()
    input.manifest.railway.staging[key] = value
    assert.throws(() => verifyStagingTarget(input), /positive integer/)
  }
})

test('accepts only the reviewed healthy staging topology', () => {
  assert.doesNotThrow(() => verifyStagingTopology(fixture()))
  for (const mutate of [
    (input) => { input.services.find(({ id }) => id === 'api').replicas.running = 2 },
    (input) => { input.services.find(({ id }) => id === 'router').replicas.configured = 2 },
    (input) => { input.services.find(({ id }) => id === 'worker').status = 'CRASHED' },
    (input) => { input.services.find(({ id }) => id === 'cache').replicas.crashed = 1 },
    (input) => { input.services.find(({ id }) => id === 'media').replicas.running = 0 },
    (input) => { input.services.find(({ id }) => id === 'media-worker').status = 'CRASHED' },
  ]) {
    const input = fixture()
    mutate(input)
    assert.throws(() => verifyStagingTopology(input), /healthy replicas/)
  }
})

test('rejects missing or URL-shaped domains', () => {
  for (const domain of ['', 'https://api-staging.example.test/path']) {
    const input = fixture()
    input.manifest.railway.staging.apiDomain = domain
    assert.throws(() => verifyStagingTarget(input))
  }
})
