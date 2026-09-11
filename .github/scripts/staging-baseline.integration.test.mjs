import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

const pgBin = process.env.SCOPE_TEST_POSTGRES_BIN ?? '/usr/lib/postgresql/16/bin';
test('public repository baselines need no key without migrations, restore authenticated snapshots, and reject unsafe transitions', {
  skip: !existsSync(join(pgBin, 'initdb')) || process.getuid?.() === 0,
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
else if (args[0] === 'variable') console.log(JSON.stringify({DATABASE_PUBLIC_URL:process.env.TEST_DATABASE}));
else process.exit(2);
`);
  executable('maintenance', `#!/usr/bin/env node
const {execFileSync} = require('node:child_process');
if(process.argv[2] === 'fence') process.exit(0);
if(process.argv[2] === 'preflight' && process.env.TEST_SCHEMA_DRIFT === '1') process.exit(1);
const applied = execFileSync('psql',[process.env.DATABASE_URL,'-XAt','-c','SELECT version FROM seaql_migrations ORDER BY version'],{encoding:'utf8'}).trim().split('\\n');
console.log(JSON.stringify({applied,pending:[],exact:true}));
`);
  executable('gh', `#!/usr/bin/env node
const {readFileSync,appendFileSync}=require('node:fs');
const path=process.argv.find(a=>a.startsWith('repos/'));
appendFileSync(process.env.TEST_GH_TRACE,path+'\\n');
if(path === 'repos/scope-vcs/scope-vcs') { console.log('false'); process.exit(0); }
if(path.endsWith('/zip')) process.stdout.write(readFileSync(process.env.TEST_ARCHIVE));
else if(path.includes('/artifacts?')) console.log('12');
else if(path.endsWith('/artifacts/12')) console.log('34');
else console.log('true');
`);
  const binary = join(root, 'bin/maintenance');
  writeFileSync(join(root, 'prepared.json'), JSON.stringify({ schemaVersion: 1, sourceSha: 'a'.repeat(40), components: {},
    maintenanceSha256: createHash('sha256').update(readFileSync(binary)).digest('hex') }));
  const plan = { metadataRestoreSafe: true, applied: ['m0001_initial'], pending: [{ name: 'm0002_metadata' }], exact: false };
  writeFileSync(join(root, 'production.json'), JSON.stringify(plan));
  const env = { ...process.env, PATH: `${join(root, 'bin')}:${process.env.PATH}`, RAILWAY_TOKEN: 'test', RAILWAY_API_TOKEN: '',
    TEST_GH_TRACE: join(root, 'gh-trace'), TEST_DATABASE: database, TEST_ARCHIVE: join(root, 'snapshot.zip'), GITHUB_REPOSITORY: 'scope-vcs/scope-vcs',
    GITHUB_OUTPUT: join(root, 'output'), SCOPE_DEPLOYMENT_MANIFEST: join(root, 'manifest.json'),
    SCOPE_MAINTENANCE_BINARY: binary, SCOPE_PREPARED_RELEASE_PATH: join(root, 'prepared.json'),
    SCOPE_PRODUCTION_MIGRATION_PLAN: join(root, 'production.json'), SCOPE_STAGING_BASELINE_DIR: join(root, 'baseline') };
  const run = () => spawnSync('bash', ['.github/scripts/staging-baseline.sh'], { env, encoding: 'utf8', timeout: 20_000 });
  // This repository is public. Matching ledgers with no migrations neither read
  // archive metadata nor require an encryption secret or publish an artifact.
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
  // Authentication succeeds, but malformed archive contents still cannot leak
  // the temporary plaintext or change the database when pg_restore fails.
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
  writeFileSync(join(root, 'production.json'), JSON.stringify({ ...plan, metadataRestoreSafe: false, pending: [{ name: 'm0003_external_storage' }] }));
  result = run();
  assert.equal(result.status, 0, result.stderr);
  assert.equal(JSON.parse(readFileSync(metadataPath)).metadataRestoreSafe, false);
  snapshot();
  sql("INSERT INTO seaql_migrations VALUES ('m0003_external_storage')");
  result = run();
  assert.notEqual(result.status, 0);
  assert.match(sql('SELECT version FROM seaql_migrations'), /m0003_external_storage/);
});
