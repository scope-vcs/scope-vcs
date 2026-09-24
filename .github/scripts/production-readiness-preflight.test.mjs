import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';
import { RAILWAY_COMPONENTS } from './deployment-components.mjs';
import { deployedReadinessBaseline, productionReadinessAudit } from './production-readiness-preflight.mjs';

const root = resolve(import.meta.dirname, '../..');

test('preflight consumes receipt JSON even inside an Actions step with GITHUB_OUTPUT', (t) => {
  const dir = mkdtempSync(join(tmpdir(), 'production-readiness-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const bin = join(dir, 'bin');
  mkdirSync(bin);
  const output = join(dir, 'github-output');
  writeFileSync(output, 'untouched\n');
  const mockFetch = join(dir, 'fetch.cjs');
  writeFileSync(mockFetch, `global.fetch = async url => {
    if (!String(url).startsWith('https://api.github.com/repos/test/repo/deployments?')) throw Error('Unexpected request');
    return new Response('[]', {status:200});
  };\n`);
  // Run the real receipt reader and role SQL generator. Only remote health and
  // provider transport are replaced; this catches the owner's Actions/stdout contract.
  writeFileSync(join(bin, 'node'), `#!${process.execPath}
const {spawnSync} = require('node:child_process');
const {writeFileSync} = require('node:fs');
if (process.argv[2].endsWith('/production-readiness-preflight.mjs')) {
  const receipts = JSON.parse(process.env.SCOPE_PRODUCTION_DEPLOYMENTS_JSON);
  if (!Object.hasOwn(receipts, 'api')) throw Error('Missing production receipt map');
  writeFileSync(process.env.HEALTH_TRACE, JSON.stringify(receipts));
  process.stdout.write('BEGIN READ ONLY; COMMIT;');
} else {
  const child = spawnSync(process.execPath, process.argv.slice(2), {stdio:'inherit'});
  process.exit(child.status ?? 1);
}
`, { mode: 0o755 });
  writeFileSync(join(bin, 'railway'), `#!/bin/bash
set -euo pipefail
case "$1" in
  status) printf '{"fixture":true}\\n' ;;
  ssh) cat > "$SQL_TRACE" ;;
  *) exit 2 ;;
esac
`, { mode: 0o755 });
  const result = spawnSync('bash', ['.github/scripts/production-readiness-preflight.sh'], {
    cwd: root, encoding: 'utf8', timeout: 10_000,
    env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, NODE_OPTIONS: `--require=${mockFetch}`,
      GITHUB_TOKEN: 'test', GITHUB_REPOSITORY: 'test/repo', GITHUB_OUTPUT: output,
      SCOPE_DEPLOYMENT_MANIFEST: join(root, '.github/deployment-services.json'),
      SCOPE_RAILWAY_SSH_PRIVATE_KEY: '', SCOPE_RAILWAY_SSH_IDENTITY_FILE: '',
      HEALTH_TRACE: join(dir, 'health.json'), SQL_TRACE: join(dir, 'audit.sql') },
  });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(readFileSync(output, 'utf8'), 'untouched\n');
  assert(existsSync(join(dir, 'health.json')));
  assert.match(readFileSync(join(dir, 'audit.sql'), 'utf8'), /BEGIN READ ONLY;/);
  assert.match(result.stdout, /database roles are ready/);
});


test('readiness verifies deployed configuration instead of blocking candidate configuration changes', () => {
  const manifest = JSON.parse(readFileSync(join(root, '.github/deployment-services.json')));
  const deployedManifest = structuredClone(manifest);
  const configs = {};
  const receipts = {};
  const instances = [];
  const oldPolicy = 'deployed role policy';
  for (const component of RAILWAY_COMPONENTS) {
    const sha = (component === 'web' ? 'b' : 'a').repeat(40);
    receipts[component] = { provider: 'railway', sourceSha: sha, evidenceId: `live-${component}`,
      artifactDigest: `sha256:${'c'.repeat(64)}` };
    const path = `old-config/${component}.json`;
    deployedManifest.services[component].deployment.runtimeConfig = path;
    deployedManifest.services[component].id = `old-${component}`;
    configs[`${sha}:${path}`] = { deploy: { healthcheckPath: '/readyz', healthcheckTimeout: 60,
      overlapSeconds: 30, drainingSeconds: 30, numReplicas: 1 } };
    instances.push({ node: { serviceId: `old-${component}`, serviceName: component, numReplicas: 1,
      activeDeployments: [{ id: `live-${component}`, status: 'SUCCESS',
        instances: [{ status: 'RUNNING' }],
        meta: { serviceManifest: configs[`${sha}:${path}`] } }] } });
  }
  const reads = [];
  const baseline = deployedReadinessBaseline(receipts, (sha, path) => {
    reads.push([sha, path]);
    if (path === '.github/deployment-services.json') return JSON.stringify(deployedManifest);
    if (path === 'deploy/postgres/runtime-roles.mjs') return oldPolicy;
    assert(configs[`${sha}:${path}`], `wrong source or config path: ${sha}:${path}`);
    return JSON.stringify(configs[`${sha}:${path}`]);
  });
  assert.equal(reads.filter(([, path]) => path === '.github/deployment-services.json').length, 2);
  const state = { deployments: receipts, manifest, baseline, candidateRolePolicy: oldPolicy,
    status: { environments: { edges: [{ node: { id: manifest.environments.production.environmentId,
      serviceInstances: { edges: instances } } }] } } };
  const sql = productionReadinessAudit(state);
  assert.match(sql, /exact_policy := EXISTS/);
  assert.match(productionReadinessAudit({ ...state, candidateRolePolicy: 'new grants for this release' }), /exact_policy := false;/);
  const effective = instances.find(({ node }) => node.serviceName === 'api').node.activeDeployments[0].meta.serviceManifest.deploy;
  effective.healthcheckTimeout = 123;
  assert.throws(() => productionReadinessAudit(state), /expected 60/);
  effective.healthcheckTimeout = 60;
  instances[0].node.activeDeployments[0].instances = [];
  assert.throws(() => productionReadinessAudit(state), /0\/1 running replicas/);
});
