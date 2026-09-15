import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdtempSync, readFileSync, writeFileSync, rmSync, mkdirSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { currentApiManifest } from './maintenance-runtime.mjs';
import { verifyPrivateImagePackage } from './railway-artifact.mjs';

const manifest = JSON.parse(readFileSync(new URL('../deployment-services.json', import.meta.url)));
const repository = 'scope-vcs/scope-vcs';
const sourceSha = 'a'.repeat(40);
const image = `ghcr.io/${repository}/railway-private-api@sha256:${'b'.repeat(64)}`;
const binary = 'verified-current-maintenance';
const maintenanceSha256 = createHash('sha256').update(binary).digest('hex');
const source = { image, sourceSha, maintenanceSha256 };
const deployment = { id: 'current', serviceId: manifest.services.api.id, environmentId: manifest.environments.production.environmentId,
  status: 'SUCCESS', meta: { serviceManifest: { source: { image } } } };

test('runtime source must match one healthy active production API deployment', () => {
  assert.equal(currentApiManifest(manifest, repository, source, [deployment]).maintenanceSha256, maintenanceSha256);
  for (const rows of [[], [deployment, deployment], [{ ...deployment, status: 'DEPLOYING' }],
    [{ ...deployment, serviceId: manifest.railway.databaseServiceId }],
    [{ ...deployment, environmentId: manifest.environments.staging.environmentId }],
    [{ ...deployment, meta: { imageDigest: `sha256:${'c'.repeat(64)}` } }]]) {
    assert.throws(() => currentApiManifest(manifest, repository, source, rows));
  }
  assert.throws(() => currentApiManifest(manifest, repository, { ...source, image: 'ghcr.io/attacker/image@sha256:' + 'b'.repeat(64) }, [deployment]));
});

test('publishing verifies deployed binary, durable pulls and package visibility before issuing a receipt', t => {
  const root = mkdtempSync(join(tmpdir(), 'scope-maintenance-publish-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  mkdirSync(join(root, 'bin'));
  const log = join(root, 'docker.log');
  writeFileSync(join(root, 'fetch.cjs'), `global.fetch = async url => new Response(JSON.stringify(String(url).includes('/packages/') ? {name:'scope-vcs/railway-private-maintenance',package_type:'container',owner:{login:'scope-vcs'},visibility:process.env.TEST_VISIBILITY || 'private'} : {login:'scope-vcs',type:'Organization'}));`);
  writeFileSync(join(root, 'bin/railway'), `#!/usr/bin/env node\nconsole.log(${JSON.stringify(JSON.stringify([deployment]))});\n`, { mode: 0o755 });
  writeFileSync(join(root, 'bin/docker'), `#!/usr/bin/env node
const fs = require('node:fs'); const a=process.argv.slice(2);
fs.appendFileSync(process.env.TEST_DOCKER_LOG, JSON.stringify({args:a,config:process.env.DOCKER_CONFIG})+'\\n');
if(a[0]==='cp') fs.writeFileSync(a[2], process.env.TEST_BINARY);
if(a[0]==='image') console.log(process.env.TEST_IMAGE_REVISION || process.env.CURRENT_API_SOURCE_SHA);
if(a[0]==='buildx') fs.writeFileSync(a[a.indexOf('--metadata-file')+1], JSON.stringify({'containerimage.digest':'sha256:'+ 'd'.repeat(64)}));
if(a[0]==='manifest' && process.env.TEST_PULL_FAIL) process.exit(1);
`, { mode: 0o755 });
  const output = join(root, 'receipt.json');
  const env = { ...process.env, PATH: `${root}/bin:${process.env.PATH}`, GITHUB_REPOSITORY: repository,
    GITHUB_SHA: 'e'.repeat(40), GITHUB_RUN_ID: '123', GITHUB_RUN_ATTEMPT: '1', GITHUB_TOKEN: 'publish-token',
    CURRENT_API_IMAGE: image, CURRENT_API_SOURCE_SHA: sourceSha, MAINTENANCE_SHA256: maintenanceSha256,
    SCOPE_RAILWAY_REGISTRY_USERNAME: 'durable-user', SCOPE_RAILWAY_REGISTRY_PASSWORD: 'durable-secret',
    TEST_DOCKER_LOG: log, TEST_BINARY: binary, NODE_OPTIONS: `--require=${root}/fetch.cjs`, DOCKER_CONFIG: `${root}/publishing` };
  const run = extra => spawnSync('bash', ['.github/scripts/publish-maintenance-runtime.sh', output], {env:{...env,...extra}, encoding:'utf8',timeout:60000});
  for (const extra of [{TEST_BINARY:'tampered'}, {TEST_IMAGE_REVISION:'f'.repeat(40)}, {TEST_PULL_FAIL:'1'}, {TEST_VISIBILITY:'public'}]) {
    const result = run(extra);
    assert.notEqual(result.status, 0);
    assert(!existsSync(output), 'Failed verification must not issue a deployment receipt');
  }
  const result = run({});
  assert.equal(result.status, 0, result.stderr);
  const receipt = JSON.parse(readFileSync(output));
  assert.equal(receipt.apiImage, image);
  assert.equal(receipt.maintenanceSha256, maintenanceSha256);
  assert.equal(receipt.image, `ghcr.io/${repository}/railway-private-maintenance@sha256:${'d'.repeat(64)}`);
  const calls = readFileSync(log,'utf8').trim().split('\n').map(JSON.parse);
  assert(calls.filter(c=>c.args[0]==='manifest').every(c=>c.config!==env.DOCKER_CONFIG));
  assert(!readFileSync(log,'utf8').includes('durable-secret'));
});

test('runtime workflow verifies staging before production and stays outside app release activation', () => {
  const workflow = readFileSync(new URL('../workflows/maintenance-runtime.yml', import.meta.url),'utf8');
  assert.match(workflow,/workflow_dispatch:/);
  assert.match(workflow,/github.ref == 'refs\/heads\/main'/);
  assert.match(workflow,/production:\n    needs: staging/);
  assert.match(workflow,/verify-maintenance-runtime.sh staging/);
  assert.match(workflow,/packages: write/);
  assert.doesNotMatch(workflow,/deploy-backend|release-cutover|railway up/);
});


test('only initial package creation accepts missing metadata', async () => {
  const imageRepository = `ghcr.io/${repository}/railway-private-maintenance`;
  const fetchImpl = async url => String(url).includes('/packages/')
    ? new Response('{}', {status:404})
    : new Response(JSON.stringify({login:'scope-vcs',type:'Organization'}));
  assert.equal((await verifyPrivateImagePackage(imageRepository, repository, {token:'token',fetchImpl,allowMissing:true})).visibility, 'missing');
  await assert.rejects(verifyPrivateImagePackage(imageRepository, repository, {token:'token',fetchImpl}), /HTTP 404/);
});

test('SSH canary verifies the service identity, binary and private database before preflight', t => {
  const root = mkdtempSync(join(tmpdir(), 'scope-maintenance-canary-'));
  t.after(() => rmSync(root, {recursive:true,force:true}));
  mkdirSync(join(root,'bin'));
  const executable = join(root,'scope-maintenance');
  writeFileSync(executable, '#!/bin/sh\ntouch "$TEST_PREFLIGHT"\n', {mode:0o755});
  const digest = createHash('sha256').update(readFileSync(executable)).digest('hex');
  const receipt = join(root,'receipt.json');
  writeFileSync(receipt,JSON.stringify({maintenanceSha256:digest}));
  const preflight = join(root,'preflight');
  writeFileSync(join(root,'bin/railway'),`#!/usr/bin/env node
const {spawnSync}=require('node:child_process');
const args=process.argv.slice(2); const get=k=>args[args.indexOf(k)+1];
const command=args.at(-1).replaceAll('/app/bin/scope-maintenance',process.env.TEST_BINARY_PATH).replaceAll('/proc/1/status',process.env.TEST_PROCESS_STATUS);
const result=spawnSync('sh',['-c',command],{stdio:'inherit',env:{...process.env,
RAILWAY_PROJECT_ID:get('--project'),RAILWAY_ENVIRONMENT_ID:get('--environment'),RAILWAY_SERVICE_ID:get('--service')}});
process.exit(result.status ?? 1);
`,{mode:0o755});
  writeFileSync(join(root,'bin/id'),'#!/bin/sh\necho 0\n',{mode:0o755});
  const status = join(root,'process-status');
  writeFileSync(status,'Uid:\t65532\t65532\t65532\t65532\n');
  const run = database => spawnSync('bash',['.github/scripts/verify-maintenance-runtime.sh','staging',receipt], {
    env:{...process.env,PATH:`${root}/bin:${process.env.PATH}`,TEST_BINARY_PATH:executable,TEST_PROCESS_STATUS:status,TEST_PREFLIGHT:preflight,DATABASE_URL:database}, encoding:'utf8',timeout:60000,
  });
  assert.notEqual(run('postgres://scope@public.example:5432/scope').status,0);
  assert(!existsSync(preflight));
  let result=run('postgres://scope@postgres.railway.internal:5432/scope');
  assert.equal(result.status,0,result.stderr);
  rmSync(preflight);
  writeFileSync(status,'Uid:\t0\t0\t0\t0\n');
  assert.notEqual(run('postgres://scope@postgres.railway.internal:5432/scope').status,0);
  assert(!existsSync(preflight));
  writeFileSync(status,'Uid:\t65532\t65532\t65532\t65532\n');
  writeFileSync(executable,'#!/bin/sh\ntouch "$TEST_PREFLIGHT"\n# changed\n');
  result=run('postgres://scope@postgres.railway.internal:5432/scope');
  assert.notEqual(result.status,0);
  assert(!existsSync(preflight));
});
