import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
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
      projectId: 'project',
    },
    environments: {
      production: { environmentId: 'production', environmentName: 'production' },
      staging: {
        apiReplicas: 3,
        apiDomain: 'api-staging.example.test',
        cacheDomain: 'cache-staging.example.test',
        environmentId: 'staging',
        environmentName: 'staging',
        routerDomain: 'router-staging.example.test',
        routerReplicas: 1,
        webDomain: 'web-staging.example.test',
      },
    },
    services: {
      api: { id: 'api', name: 'scope-api', sourceDirectory: 'api', binary: 'scope-vcs' },
      cache: { id: 'cache', name: 'scope-cache-service', sourceDirectory: 'cache-service', binary: 'scope-cache-service' },
      'git-router': { id: 'router', name: 'scope-repo-router', sourceDirectory: 'repo-router', binary: 'scope-repo-router' },
      'media-api': { id: 'media', name: 'scope-media', sourceDirectory: 'media-service', binary: 'scope-media-service' },
      'media-worker': { id: 'media-worker', name: 'scope-media-worker', sourceDirectory: 'media-worker' },
      web: { id: 'web', name: 'scope-web', sourceDirectory: 'web' },
      'run-worker': { id: 'worker', name: 'scope-worker', sourceDirectory: 'worker', binary: 'scope-worker' },
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

function candidateCheckout(t) {
  const root = mkdtempSync(join(tmpdir(), 'scope-staging-deploy-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const scripts = join(root, '.github/scripts')
  const bin = join(root, 'bin')
  mkdirSync(scripts, { recursive: true })
  mkdirSync(bin)
  execFileSync('git', ['init', '--quiet', root])
  execFileSync('git', ['-c', 'user.name=Scope Test', '-c', 'user.email=scope@example.test',
    '-c', 'commit.gpgsign=false', 'commit', '--quiet', '--allow-empty', '-m', 'candidate'], { cwd: root })
  const candidate = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim()
  return { root, scripts, bin, candidate }
}

test('staging deploys and records the candidate checkout when the workflow revision differs', (t) => {
  const { root, scripts, bin, candidate } = candidateCheckout(t)
  const input = fixture()
  writeFileSync(join(root, '.github/deployment-services.json'), JSON.stringify(input.manifest))
  copyFileSync(new URL('./verify-staging-target.mjs', import.meta.url), join(scripts, 'verify-staging-target.mjs'))
  copyFileSync(new URL('./deploy-staging-railway.sh', import.meta.url), join(scripts, 'deploy-staging-railway.sh'))
  writeFileSync(join(scripts, 'railway-read.mjs'), `
    if (process.argv[1]?.endsWith('railway-read.mjs')) {
    const input = ${JSON.stringify(input)};
    const args = process.argv.slice(2);
    const command = args.slice(0, 2).join(' ');
    const service = args[args.indexOf('--service') + 1];
    if (args[0] === 'status') console.log(JSON.stringify(input.status));
    else if (command === 'service list') console.log(JSON.stringify(input.services));
    else if (command === 'variable list') console.log(JSON.stringify(service === 'api' ? {
      SCOPE_CACHE_URL: 'https://' + input.manifest.environments.staging.cacheDomain,
      SCOPE_GIT_PUBLIC_URL: 'https://' + input.manifest.environments.staging.routerDomain,
    } : { SCOPE_REPO_ROUTER_BACKEND: 'scope-api.railway.internal:8080', SCOPE_REPO_ROUTER_READ_REPLICAS: '3' }));
    else if (command === 'deployment list') console.log(JSON.stringify([{ id: service + '-deployment', status: 'SUCCESS' }]));
    else throw new Error('Unexpected provider read: ' + args.join(' '));
    }
  `)
  writeFileSync(join(bin, 'railway'), '#!/bin/sh\ntest "$1 $2" = "variable set"\n', { mode: 0o755 })
  writeFileSync(join(scripts, 'deploy-railway.sh'), `#!/bin/sh
    node -e 'require("node:fs").writeFileSync("deployment.json", JSON.stringify({ source: process.env.SCOPE_DEPLOYMENT_SOURCE_SHA, message: process.env.RAILWAY_DEPLOY_MESSAGE }))'
  `)
  writeFileSync(join(root, 'prepared.json'), JSON.stringify({ schemaVersion: 1, sourceSha: candidate,
    components: { web: { sourceSha: candidate, serviceId: 'web', image: `ghcr.io/scope-vcs/release-web@sha256:${'b'.repeat(64)}` } } }))
  for (const name of ['railway-artifact.mjs', 'railway-retry.mjs']) {
    copyFileSync(new URL(`./${name}`, import.meta.url), join(scripts, name))
  }
  // The artifact module imports the provider reader but this test never calls it.
  writeFileSync(join(scripts, 'railway-read.mjs'), 'export function readRailway() {}\n' + readFileSync(join(scripts, 'railway-read.mjs'), 'utf8'))
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
      SCOPE_PREPARED_RELEASE_PATH: join(root, 'prepared.json'),
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
  input.manifest.environments.staging.environmentId = 'production'
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
    input.manifest.environments.staging[key] = value
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
    input.manifest.environments.staging.apiDomain = domain
    assert.throws(() => verifyStagingTarget(input))
  }
})

for (const resume of [false, true]) {
test(`full staging ${resume ? 'resume' : 'migration'} restores API readiness before its Git router and records each participant once`, (t) => {
  const { root, scripts, bin, candidate } = candidateCheckout(t)
  const input = fixture()
  for (const service of input.services.filter(({ id }) => id !== 'database')) {
    service.status = 'STOPPED'
    service.replicas = { configured: service.id === 'api' ? 3 : 1, running: 0, crashed: 0 }
  }
  writeFileSync(join(root, 'provider.json'), JSON.stringify({ ...input, activations: [], maintenance: [] }))
  writeFileSync(join(root, '.github/deployment-services.json'), JSON.stringify(input.manifest))
  for (const name of ['deploy-staging-railway.sh', 'verify-staging-target.mjs', 'railway-artifact.mjs', 'railway-retry.mjs']) {
    copyFileSync(new URL(`./${name}`, import.meta.url), join(scripts, name))
  }
  // The lower deployment boundary models Railway readiness and immutable receipts.
  // It rejects the real failure: a Git router cannot resolve its stopped API backend.
  writeFileSync(join(root, 'provider.mjs'), `
    import assert from 'node:assert/strict';
    import { readFileSync, writeFileSync, appendFileSync } from 'node:fs';
    import { spawnSync } from 'node:child_process';
    const state = JSON.parse(readFileSync('provider.json'));
    const args = process.argv.slice(2);
    const action = args.shift();
    const serviceId = args[args.indexOf('--service') + 1];
    const save = () => writeFileSync('provider.json', JSON.stringify(state));
    if (action === 'read') {
      if (args[0] === 'status') console.log(JSON.stringify(state.status));
      else if (args[0] === 'service') console.log(JSON.stringify(state.services));
      else if (args[0] === 'variable') console.log(JSON.stringify(serviceId === 'database'
        ? { DATABASE_PUBLIC_URL: 'postgres://staging-fixture' }
        : serviceId === 'api' ? {
          SCOPE_CACHE_URL: 'https://' + state.manifest.environments.staging.cacheDomain,
          SCOPE_GIT_PUBLIC_URL: 'https://' + state.manifest.environments.staging.routerDomain,
        } : { SCOPE_REPO_ROUTER_BACKEND: 'scope-api.railway.internal:8080', SCOPE_REPO_ROUTER_READ_REPLICAS: '3' }));
      else throw new Error('Unexpected provider read');
    } else if (action === 'railway') {
      assert.equal(args[args.indexOf('--project') + 1], 'project');
      assert.equal(args[args.indexOf('--environment') + 1], 'staging');
      if (args[0] === 'run') {
        const command = args.slice(args.indexOf('--') + 1);
        const result = spawnSync(command[0], command.slice(1), { stdio: 'inherit', env: process.env });
        process.exit(result.status ?? 1);
      }
      assert.equal(args.slice(0, 2).join(' '), 'variable set');
    } else if (action === 'maintenance') {
      assert.equal(process.env.DATABASE_URL, 'postgres://staging-fixture');
      assert.ok(state.services.filter(s => ['api', 'cache', 'worker', 'media', 'media-worker'].includes(s.id))
        .every(s => s.replicas.running === 0));
      assert.ok(['plan', 'validate-workflow-catalogs', 'apply', 'backfill-landing-files', 'backfill-workflow-catalogs'].includes(args[0]));
      state.maintenance.push(args[0]);
      save();
      if (args[0] === 'plan') console.log(JSON.stringify({ exact: true, applied: ['m0001_initial'], pending: [] }));
    } else if (action === 'activate') {
      const component = process.env.SCOPE_DEPLOYMENT_COMPONENT;
      const prepared = JSON.parse(readFileSync(process.env.SCOPE_PREPARED_RELEASE_PATH));
      const artifact = prepared.components[component];
      assert.equal(args[0], artifact.serviceId);
      assert.equal(process.env.SCOPE_DEPLOYMENT_SOURCE_SHA, prepared.sourceSha);
      assert.equal(process.env.SCOPE_RAILWAY_ENVIRONMENT_ID, 'staging');
      if (process.env.SCOPE_STAGING_RESUME === '1') assert.deepEqual(state.maintenance, ['plan']);
      else assert.ok(state.maintenance.includes('apply'));
      assert.ok(!state.activations.includes(component), 'A participant was activated twice');
      if (component === 'git-router') {
        const api = state.services.find(s => s.id === 'api');
        assert.equal(api.status, 'SUCCESS', 'Git router readiness failed: API backend DNS is unavailable');
        assert.equal(api.replicas.running, api.replicas.configured);
      }
      if (component === 'media-worker') assert.equal(args[1], artifact.image);
      const service = state.services.find(s => s.id === artifact.serviceId);
      service.status = 'SUCCESS';
      service.replicas.running = service.replicas.configured;
      state.activations.push(component);
      save();
      appendFileSync(process.env.SCOPE_DEPLOYMENT_EVIDENCE_PATH, JSON.stringify({
        component, sourceSha: prepared.sourceSha, provider: 'railway', evidenceId: 'new-' + artifact.serviceId,
      }) + '\\n');
    } else throw new Error('Unexpected provider action');
  `)
  writeFileSync(join(scripts, 'railway-read.mjs'), `
    import { execFileSync } from 'node:child_process';
    export function readRailway() { throw new Error('Unexpected artifact provider call'); }
    if (process.argv[1]?.endsWith('railway-read.mjs'))
      process.stdout.write(execFileSync(process.execPath, ['provider.mjs', 'read', ...process.argv.slice(2)]));
  `)
  writeFileSync(join(bin, 'railway'), '#!/bin/sh\nexec node provider.mjs railway "$@"\n', { mode: 0o755 })
  const binary = join(bin, 'maintenance')
  writeFileSync(binary, '#!/bin/sh\nexec node provider.mjs maintenance "$@"\n', { mode: 0o755 })
  writeFileSync(join(scripts, 'deploy-railway.sh'), '#!/bin/sh\nexec node provider.mjs activate "$@"\n')
  writeFileSync(join(scripts, 'deploy-railway-image.mjs'), `
    import { execFileSync } from 'node:child_process';
    execFileSync(process.execPath, ['provider.mjs', 'activate', ...process.argv.slice(2)], { stdio: 'inherit' });
  `)
  const components = Object.fromEntries(Object.entries(input.manifest.services).map(([component, service]) => [component, {
    sourceSha: candidate, serviceId: service.id, image: `ghcr.io/scope-vcs/release-${component}@sha256:${'b'.repeat(64)}`,
  }]))
  writeFileSync(join(root, 'prepared.json'), JSON.stringify({ schemaVersion: 1, sourceSha: candidate, components,
    maintenanceSha256: createHash('sha256').update(readFileSync(binary)).digest('hex') }))
  const result = spawnSync('bash', [join(scripts, 'deploy-staging-railway.sh')], {
    cwd: root, encoding: 'utf8', timeout: 15_000,
    env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, RAILWAY_TOKEN: 'staging-token', RAILWAY_API_TOKEN: '',
      SCOPE_STAGING_RESUME: resume ? '1' : '0',
      SCOPE_MAINTENANCE_BINARY: binary, SCOPE_DEPLOYMENT_MANIFEST: join(root, '.github/deployment-services.json'),
      SCOPE_PREPARED_RELEASE_PATH: join(root, 'prepared.json'), SCOPE_STAGING_EVIDENCE_PATH: join(root, 'evidence.json') },
  })
  assert.ifError(result.error)
  assert.equal(result.status, 0, result.stderr)
  const state = JSON.parse(readFileSync(join(root, 'provider.json')))
  assert.deepEqual([...state.activations].sort(), Object.keys(components).sort())
  assert.ok(state.activations.indexOf('api') < state.activations.indexOf('git-router'))
  assert.doesNotThrow(() => verifyStagingTopology(state))
  assert.deepEqual(JSON.parse(readFileSync(join(root, 'evidence.json'))), {
    commit: candidate, environmentId: 'staging', candidateDeployments: 1,
    deployments: state.activations.map(component => ({ service: components[component].serviceId,
      deploymentId: 'new-' + components[component].serviceId, status: 'SUCCESS' })),
  })
})
}
