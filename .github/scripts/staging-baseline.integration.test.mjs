import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { localClusterSkip, pgBin } from '../../deploy/postgres/test-cluster.mjs';

test('public repository baselines need no key without migrations, restore authenticated snapshots, and reject unsafe transitions', {
  skip: localClusterSkip,
}, (t) => {
  const root = mkdtempSync(join(tmpdir(), 'scope-baseline-test-'));
  const command = (binary, args, options = {}) => execFileSync(binary, args, { encoding: 'utf8', stdio: 'pipe', ...options });
  command(join(pgBin, 'initdb'), ['-D', join(root, 'db'), '-A', 'trust', '--no-locale']);
  command(join(pgBin, 'pg_ctl'), ['-D', join(root, 'db'), '-o', `-k ${root} -p 55439 -c listen_addresses=`, '-l', join(root, 'log'), 'start']);
  t.after(() => {
    spawnSync(join(pgBin, 'pg_ctl'), ['-D', join(root, 'db'), 'stop', '-m', 'immediate']);
    rmSync(root, { recursive: true, force: true });
  });
  const database = `host=${root} port=55439 dbname=postgres`;
  const sql = (statement) => command('psql', [database, '-XAt', '-v', 'ON_ERROR_STOP=1', '-c', statement]).trim();
  sql("CREATE TABLE seaql_migrations(version text); INSERT INTO seaql_migrations VALUES ('m0001_initial'); CREATE TABLE scope_repositories(id text); INSERT INTO scope_repositories VALUES ('existing-repo')");
  mkdirSync(join(root, 'bin'));
  const executable = (name, body) => writeFileSync(join(root, 'bin', name), body, { mode: 0o755 });
  const manifest = JSON.parse(readFileSync(new URL('../deployment-services.json', import.meta.url)));
  writeFileSync(join(root, 'manifest.json'), JSON.stringify(manifest));
  executable('railway', `#!/usr/bin/env node
const m = ${JSON.stringify(manifest)};
const args = process.argv.slice(2);
if (args[0] === 'status') console.log(JSON.stringify({id:m.railway.projectId,environments:{edges:[{node:{id:m.environments.staging.environmentId,name:m.environments.staging.environmentName}}]}}));
else if (args[0] === 'service') console.log(JSON.stringify([...Object.values(m.services),{id:m.railway.databaseServiceId,name:'scope-postgres'}]));
else if (args[0] === 'ssh') {
  const {spawnSync} = require('node:child_process');
  const result = spawnSync('sh', ['-c', args[args.indexOf('--') + 1]], {stdio:'inherit', env:{...process.env,
    DATABASE_URL:process.env.TEST_DATABASE, RAILWAY_PROJECT_ID:m.railway.projectId,
    RAILWAY_ENVIRONMENT_ID:m.environments.staging.environmentId,
    RAILWAY_SERVICE_ID:process.env.SCOPE_RAILWAY_MAINTENANCE_SERVICE_ID}});
  process.exit(result.status ?? 1);
}
else process.exit(2);
`);
  executable('maintenance', `#!/usr/bin/env node
const {execFileSync} = require('node:child_process');
if(process.argv[2] === 'fence') process.exit(0);
if(process.argv[2] === 'preflight' && process.env.TEST_SCHEMA_DRIFT === '1') process.exit(1);
const applied = execFileSync('psql',[process.env.DATABASE_URL,'-XAt','-c','SELECT version FROM seaql_migrations ORDER BY version'],{encoding:'utf8'}).trim().split('\\n');
console.log(JSON.stringify({applied,pending:[],exact:true}));
`);
  const runCreatedAt = '2026-01-01T00:00:00Z';
  const runCreatedUnix = Date.parse(runCreatedAt) / 1000;
  const baselineName = `staging-baseline-${createHash('sha256').update(JSON.stringify(['m0001_initial'])).digest('hex')}`;
  executable('gh', `#!/usr/bin/env node
const {readFileSync,appendFileSync}=require('node:fs');
const {execFileSync}=require('node:child_process');
const args=process.argv.slice(2);
const path=args.find(a=>a.startsWith('repos/'));
appendFileSync(process.env.TEST_GH_TRACE,path+'\\n');
if(path === 'repos/scope-vcs/scope-vcs') { console.log('false'); process.exit(0); }
if(path.endsWith('/zip')) { process.stdout.write(readFileSync(process.env.TEST_ARCHIVE)); process.exit(0); }
const run={id:34,head_branch:'main'};
const artifact={id:12,name:${JSON.stringify(baselineName)},expired:false,workflow_run:run};
const responses={
  'repos/scope-vcs/scope-vcs/actions/artifacts/12':artifact,
  'repos/scope-vcs/scope-vcs/actions/runs/34':{path:'.github/workflows/release.yml',head_branch:'main',event:'workflow_dispatch',created_at:${JSON.stringify(runCreatedAt)}},
};
const body=path.includes('/artifacts?') ? {artifacts:[artifact]} : responses[path];
if(!body) process.exit(1);
const filter=args[args.indexOf('--jq')+1];
process.stdout.write(execFileSync('jq',['-r',filter],{input:JSON.stringify(body),encoding:'utf8'}));
`);
  const binary = join(root, 'bin/maintenance');
  const prepared = (preparationRunId) => writeFileSync(join(root, 'prepared.json'), JSON.stringify({ schemaVersion: 1,
    sourceSha: 'a'.repeat(40), components: {}, preparationRunId,
    maintenanceSha256: createHash('sha256').update(readFileSync(binary)).digest('hex') }));
  prepared('34');
  const plan = { metadataRestoreSafe: true, applied: ['m0001_initial'], pending: [{ name: 'm0002_metadata' }], exact: false };
  writeFileSync(join(root, 'production.json'), JSON.stringify(plan));
  const env = { ...process.env, PATH: `${join(root, 'bin')}:${pgBin}:${process.env.PATH}`, RAILWAY_TOKEN: 'test', RAILWAY_API_TOKEN: '',
    TEST_GH_TRACE: join(root, 'gh-trace'), TEST_DATABASE: database, TEST_ARCHIVE: join(root, 'snapshot.zip'), GITHUB_REPOSITORY: 'scope-vcs/scope-vcs',
    GITHUB_OUTPUT: join(root, 'output'), SCOPE_DEPLOYMENT_MANIFEST: join(root, 'manifest.json'),
    SCOPE_RAILWAY_MAINTENANCE_SERVICE_ID: '11111111-1111-1111-1111-111111111111', SCOPE_MAINTENANCE_BINARY: binary, SCOPE_PREPARED_RELEASE_PATH: join(root, 'prepared.json'),
    SCOPE_PRODUCTION_MIGRATION_PLAN: join(root, 'production.json'), SCOPE_STAGING_BASELINE_DIR: join(root, 'baseline') };
  const run = () => spawnSync('bash', ['.github/scripts/staging-baseline.sh'], { env, encoding: 'utf8', timeout: 60_000 });
  delete env.SCOPE_STAGING_BASELINE_KEY;
  writeFileSync(join(root, 'production.json'), JSON.stringify({ ...plan, pending: [], exact: true }));
  let result = run();
  assert.equal(result.status, 0, result.stderr);
  assert.equal(existsSync(env.TEST_GH_TRACE), false);
  assert.equal(existsSync(env.GITHUB_OUTPUT), false);
  assert.equal(existsSync(join(env.SCOPE_STAGING_BASELINE_DIR, 'database.dump.enc')), false);
  env.TEST_SCHEMA_DRIFT = '1';
  result = run();
  assert.notEqual(result.status, 0, 'Matching ledgers must still reject schema drift');
  assert.equal(existsSync(join(env.SCOPE_STAGING_BASELINE_DIR, 'database.dump.enc')), false);
  delete env.TEST_SCHEMA_DRIFT;
  writeFileSync(join(root, 'production.json'), JSON.stringify(plan));
  result = run();
  assert.notEqual(result.status, 0, 'A pending migration must not retain an unencrypted snapshot');
  assert.equal(existsSync(join(env.SCOPE_STAGING_BASELINE_DIR, 'database.dump')), false);
  env.SCOPE_STAGING_BASELINE_KEY = randomBytes(32).toString('hex');
  const undeclaredPlan = { ...plan };
  delete undeclaredPlan.metadataRestoreSafe;
  writeFileSync(join(root, 'production.json'), JSON.stringify(undeclaredPlan));
  result = run();
  assert.notEqual(result.status, 0, 'A migration plan must declare its restore implications');
  assert.match(result.stderr, /must declare metadataRestoreSafe/);
  assert.equal(existsSync(join(env.SCOPE_STAGING_BASELINE_DIR, 'database.dump')), false);
  writeFileSync(join(root, 'production.json'), JSON.stringify(plan));
  result = run();
  assert.equal(result.status, 0, result.stderr);
  const encryptedPath = join(env.SCOPE_STAGING_BASELINE_DIR, 'database.dump.enc');
  const checksumPath = join(env.SCOPE_STAGING_BASELINE_DIR, 'database.sha256');
  assert.equal(existsSync(join(env.SCOPE_STAGING_BASELINE_DIR, 'database.dump')), false);
  assert.match(readFileSync(checksumPath, 'utf8'), /^[a-f0-9]{64}  database\.dump\.enc\n$/);
  const snapshot = () => {
    rmSync(env.TEST_ARCHIVE, { force: true });
    command('zip', ['-q', env.TEST_ARCHIVE, 'database.dump.enc', 'database.sha256', 'baseline.json'], { cwd: env.SCOPE_STAGING_BASELINE_DIR });
  };
  snapshot();
  sql("INSERT INTO seaql_migrations VALUES ('m0002_metadata'); ALTER TABLE scope_repositories ADD COLUMN candidate text; CREATE TABLE candidate_only(id int)");
  const originalKey = env.SCOPE_STAGING_BASELINE_KEY;
  env.SCOPE_STAGING_BASELINE_KEY = randomBytes(32).toString('hex');
  result = run();
  assert.notEqual(result.status, 0, 'Wrong keys must fail before database restore');
  assert.match(sql('SELECT version FROM seaql_migrations'), /m0002_metadata/);
  assert.equal(existsSync(join(env.SCOPE_STAGING_BASELINE_DIR, 'restore')), false);
  env.SCOPE_STAGING_BASELINE_KEY = originalKey;
  const authenticCiphertext = readFileSync(encryptedPath);
  const corruptedCiphertext = Buffer.from(authenticCiphertext);
  corruptedCiphertext[100] ^= 1;
  writeFileSync(encryptedPath, corruptedCiphertext);
  writeFileSync(checksumPath, `${createHash('sha256').update(corruptedCiphertext).digest('hex')}  database.dump.enc\n`);
  snapshot();
  result = run();
  assert.notEqual(result.status, 0, 'A recomputed public checksum cannot authorize tampered ciphertext');
  assert.match(sql('SELECT version FROM seaql_migrations'), /m0002_metadata/);
  assert.equal(sql("SELECT count(*) FROM information_schema.tables WHERE table_name='candidate_only'"), '1');
  assert.equal(existsSync(join(env.SCOPE_STAGING_BASELINE_DIR, 'restore')), false);
  const invalidDump = join(root, 'invalid.dump');
  writeFileSync(invalidDump, 'not a PostgreSQL archive');
  command(process.execPath, ['.github/scripts/staging-baseline-crypto.mjs', 'encrypt', invalidDump,
    encryptedPath, join(env.SCOPE_STAGING_BASELINE_DIR, 'baseline.json')], { env });
  writeFileSync(checksumPath, `${createHash('sha256').update(readFileSync(encryptedPath)).digest('hex')}  database.dump.enc\n`);
  snapshot();
  result = run();
  assert.notEqual(result.status, 0, 'Invalid authenticated archives must fail closed');
  assert.match(sql('SELECT version FROM seaql_migrations'), /m0002_metadata/);
  assert.equal(existsSync(join(env.SCOPE_STAGING_BASELINE_DIR, 'restore')), false);
  writeFileSync(encryptedPath, authenticCiphertext);
  writeFileSync(checksumPath, `${createHash('sha256').update(authenticCiphertext).digest('hex')}  database.dump.enc\n`);
  snapshot();
  result = run();
  assert.equal(result.status, 0, result.stderr);
  assert.equal(existsSync(join(env.SCOPE_STAGING_BASELINE_DIR, 'restore')), false);
  assert.equal(existsSync(join(env.SCOPE_STAGING_BASELINE_DIR, 'database.dump')), false);
  assert.equal(sql('SELECT id FROM scope_repositories'), 'existing-repo');
  assert.equal(sql('SELECT version FROM seaql_migrations'), 'm0001_initial');
  assert.equal(sql("SELECT count(*) FROM information_schema.tables WHERE table_name='candidate_only'"), '0');
  assert.equal(sql("SELECT count(*) FROM information_schema.columns WHERE table_name='scope_repositories' AND column_name='candidate'"), '0');
  const metadataPath = join(env.SCOPE_STAGING_BASELINE_DIR, 'baseline.json');
  sql(`CREATE TABLE scope_orphan_object_jobs(object_key varchar, updated_at_unix bigint, next_run_at_unix bigint,
      attempts int, completed_at_unix bigint);
    CREATE TABLE scope_object_references(object_key varchar, ref_kind varchar, ref_id varchar);
    CREATE TABLE scope_repo_storage_cleanup_jobs(repo_id varchar, generation varchar, incarnation_id varchar,
      updated_at_unix bigint, next_run_at_unix bigint, attempts int, completed_at_unix bigint);
    CREATE TABLE scope_request_media_attachments(id text);
    CREATE TABLE scope_request_media_cleanup_jobs(attachment_id text, updated_at_unix bigint, attempt int, completed_at_unix bigint,
      state text DEFAULT 'Queued', lease_token text, available_at_unix bigint DEFAULT 0);
    ALTER TABLE scope_repositories ADD COLUMN incarnation_id varchar;
    UPDATE scope_repositories SET incarnation_id = 'current';
    INSERT INTO scope_orphan_object_jobs VALUES ('queued-blob', ${runCreatedUnix - 60}, ${runCreatedUnix + 540}, 0, NULL),
      ('referenced-blob', ${runCreatedUnix - 60}, ${runCreatedUnix + 540}, 0, NULL);
    INSERT INTO scope_object_references VALUES ('referenced-blob', 'request_snapshot', 'req_1');
    INSERT INTO scope_request_media_attachments VALUES ('deleted-request-media'), ('live-media');
    INSERT INTO scope_request_media_cleanup_jobs VALUES ('deleted-request-media', ${runCreatedUnix - 60}, 1, NULL)`);
  writeFileSync(join(root, 'production.json'), JSON.stringify({ ...plan, metadataRestoreSafe: false, pending: [{ name: 'm0003_external_storage' }] }));
  result = run();
  assert.equal(result.status, 0, result.stderr);
  assert.equal(JSON.parse(readFileSync(metadataPath)).metadataRestoreSafe, false);
  snapshot();
  sql("INSERT INTO seaql_migrations VALUES ('m0003_external_storage')");
  result = run();
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /not metadata-restore-safe/);
  assert.match(sql('SELECT version FROM seaql_migrations ORDER BY version'), /m0003_external_storage/);

  const provision = () => spawnSync('bash', ['.github/scripts/staging-baseline.sh', 'provision', '12'],
    { env, encoding: 'utf8', timeout: 60_000 });
  const deleted = runCreatedUnix + 60;
  sql(`UPDATE scope_orphan_object_jobs SET attempts = 1, completed_at_unix = ${deleted}, updated_at_unix = ${deleted}
      WHERE object_key = 'queued-blob';
    UPDATE scope_request_media_cleanup_jobs SET attempt = 2, completed_at_unix = ${deleted}, updated_at_unix = ${deleted};
    INSERT INTO scope_request_media_cleanup_jobs VALUES ('post-snapshot-smoke-media', ${deleted}, 1, ${deleted});
    INSERT INTO scope_repo_storage_cleanup_jobs VALUES ('existing-repo', 'old', 'retired', ${deleted}, ${deleted}, 1, ${deleted})`);
  const refusedAfter = (deletion, undo, reason) => {
    sql(deletion);
    result = provision();
    assert.notEqual(result.status, 0, reason);
    assert.match(result.stderr, /Objects live in this snapshot were deleted after it was taken/);
    assert.match(sql('SELECT version FROM seaql_migrations ORDER BY version'), /m0003_external_storage/);
    sql(undo);
  };
  refusedAfter(`UPDATE scope_orphan_object_jobs SET attempts = 1, completed_at_unix = ${deleted}, updated_at_unix = ${deleted}
      WHERE object_key = 'referenced-blob'`,
    "UPDATE scope_orphan_object_jobs SET attempts = 0, completed_at_unix = NULL WHERE object_key = 'referenced-blob'",
    'Provisioning must refuse a snapshot whose referenced blob was deleted, even with a pending cleanup job');
  refusedAfter(`UPDATE scope_orphan_object_jobs SET updated_at_unix = ${deleted}, next_run_at_unix = ${deleted + 300}
      WHERE object_key = 'referenced-blob'`,
    `UPDATE scope_orphan_object_jobs SET next_run_at_unix = ${deleted + 600} WHERE object_key = 'referenced-blob'`,
    'Provisioning must refuse a snapshot whose referenced blob was claimed for deletion');
  refusedAfter(`INSERT INTO scope_request_media_cleanup_jobs VALUES ('live-media', ${deleted}, 1, ${deleted})`,
    "DELETE FROM scope_request_media_cleanup_jobs WHERE attachment_id = 'live-media'",
    'Provisioning must refuse a snapshot after its live media was deleted');
  refusedAfter(`INSERT INTO scope_repo_storage_cleanup_jobs VALUES ('existing-repo', 'live', 'current', ${deleted}, ${deleted + 300}, 0, NULL)`,
    "DELETE FROM scope_repo_storage_cleanup_jobs WHERE generation = 'live'",
    'Provisioning must refuse a snapshot after its live repository storage was claimed for deletion');
  sql(`UPDATE scope_orphan_object_jobs SET updated_at_unix = ${deleted}, next_run_at_unix = ${deleted + 600}
      WHERE object_key = 'referenced-blob';
    INSERT INTO scope_repo_storage_cleanup_jobs VALUES ('existing-repo', 'deferred', 'current', ${deleted}, ${deleted}, 0, NULL);
    INSERT INTO scope_request_media_cleanup_jobs (attachment_id, updated_at_unix, attempt, completed_at_unix)
      VALUES ('live-media', ${deleted}, 0, NULL)`);
  const processedNextRun = sql("SELECT next_run_at_unix FROM scope_orphan_object_jobs WHERE object_key = 'queued-blob'");
  const deferral = command('bash', ['-c', 'source .github/scripts/staging-deletion-window.sh && defer_staging_deletions_sql "$1"',
    'defer', String(runCreatedUnix)]);
  sql(deferral);
  const deletionWindow = Number(command('bash', ['-c', 'source .github/scripts/staging-deletion-window.sh && echo $staging_deletion_window_seconds']));
  assert.equal(sql("SELECT next_run_at_unix - updated_at_unix FROM scope_orphan_object_jobs WHERE object_key = 'referenced-blob'"), String(deletionWindow));
  assert.equal(sql("SELECT next_run_at_unix - updated_at_unix FROM scope_repo_storage_cleanup_jobs WHERE generation = 'deferred'"), String(deletionWindow));
  assert.equal(sql("SELECT available_at_unix - updated_at_unix FROM scope_request_media_cleanup_jobs WHERE attachment_id = 'live-media'"), String(deletionWindow));
  assert.equal(sql("SELECT next_run_at_unix FROM scope_orphan_object_jobs WHERE object_key = 'queued-blob'"), processedNextRun,
    'Deferral must leave processed jobs unchanged');
  prepared('35');
  result = provision();
  assert.notEqual(result.status, 0, 'Provisioning must run only the baseline run\'s maintenance binary');
  assert.match(result.stderr, /must come from baseline run 34/);
  prepared('34');
  result = provision();
  assert.equal(result.status, 0, result.stderr);
  assert.equal(sql('SELECT version FROM seaql_migrations'), 'm0001_initial');
  assert.equal(sql("SELECT string_agg(object_key, ',' ORDER BY object_key) FROM scope_orphan_object_jobs WHERE completed_at_unix IS NULL"), 'queued-blob,referenced-blob');
  assert.equal(sql('SELECT id FROM scope_repositories'), 'existing-repo');
  assert.equal(existsSync(join(env.SCOPE_STAGING_BASELINE_DIR, 'restore')), false);
});
