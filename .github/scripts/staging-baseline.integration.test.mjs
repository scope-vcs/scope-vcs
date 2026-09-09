import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

const pgBin = process.env.SCOPE_TEST_POSTGRES_BIN ?? '/usr/lib/postgresql/16/bin';
test('staging snapshot restores the baseline atomically and rejects unsafe storage transitions', {
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
const applied = execFileSync('psql',[process.env.DATABASE_URL,'-XAt','-c','SELECT version FROM seaql_migrations ORDER BY version'],{encoding:'utf8'}).trim().split('\\n');
console.log(JSON.stringify({applied,pending:[],exact:true}));
`);
  executable('gh', `#!/usr/bin/env node
const {readFileSync}=require('node:fs');
const path=process.argv.find(a=>a.startsWith('repos/'));
if(path.endsWith('/zip')) process.stdout.write(readFileSync(process.env.TEST_ARCHIVE));
else if(path.includes('/artifacts?')) console.log('12');
else if(path.endsWith('/artifacts/12')) console.log('34');
else console.log('true');
`);
  const binary = join(root, 'bin/maintenance');
  writeFileSync(join(root, 'prepared.json'), JSON.stringify({ schemaVersion: 1, sourceSha: 'a'.repeat(40), components: {},
    maintenanceSha256: createHash('sha256').update(readFileSync(binary)).digest('hex') }));
  const plan = { applied: ['m0001_initial'], pending: [{ name: 'm0002_metadata' }], exact: false };
  writeFileSync(join(root, 'production.json'), JSON.stringify(plan));
  const env = { ...process.env, PATH: `${join(root, 'bin')}:${process.env.PATH}`, RAILWAY_TOKEN: 'test', RAILWAY_API_TOKEN: '',
    TEST_DATABASE: database, TEST_ARCHIVE: join(root, 'snapshot.zip'), GITHUB_REPOSITORY: 'scope-vcs/scope-vcs',
    GITHUB_OUTPUT: join(root, 'output'), SCOPE_DEPLOYMENT_MANIFEST: join(root, 'manifest.json'),
    SCOPE_MAINTENANCE_BINARY: binary, SCOPE_PREPARED_RELEASE_PATH: join(root, 'prepared.json'),
    SCOPE_PRODUCTION_MIGRATION_PLAN: join(root, 'production.json'), SCOPE_STAGING_BASELINE_DIR: join(root, 'baseline') };
  const run = () => spawnSync('bash', ['.github/scripts/staging-baseline.sh'], { env, encoding: 'utf8', timeout: 20_000 });
  let result = run();
  assert.equal(result.status, 0, result.stderr);
  const snapshot = () => {
    rmSync(env.TEST_ARCHIVE, { force: true });
    command('zip', ['-q', env.TEST_ARCHIVE, 'database.dump', 'database.sha256', 'baseline.json'], { cwd: env.SCOPE_STAGING_BASELINE_DIR });
  };
  snapshot();
  sql("INSERT INTO seaql_migrations VALUES ('m0002_metadata'); ALTER TABLE scope_repositories ADD COLUMN candidate text; CREATE TABLE candidate_only(id int)");
  result = run();
  assert.equal(result.status, 0, result.stderr);
  assert.equal(sql('SELECT id FROM scope_repositories'), 'existing-repo');
  assert.equal(sql('SELECT version FROM seaql_migrations'), 'm0001_initial');
  assert.equal(sql("SELECT count(*) FROM information_schema.tables WHERE table_name='candidate_only'"), '0');
  assert.equal(sql("SELECT count(*) FROM information_schema.columns WHERE table_name='scope_repositories' AND column_name='candidate'"), '0');
  const metadataPath = join(env.SCOPE_STAGING_BASELINE_DIR, 'baseline.json');
  writeFileSync(join(root, 'production.json'), JSON.stringify({ ...plan, pending: [{ name: 'm0033_git_segment_streaming_v2' }] }));
  result = run();
  assert.equal(result.status, 0, result.stderr);
  assert.equal(JSON.parse(readFileSync(metadataPath)).metadataRestoreSafe, false);
  snapshot();
  sql("INSERT INTO seaql_migrations VALUES ('m0003_external_storage')");
  result = run();
  assert.notEqual(result.status, 0);
  assert.match(sql('SELECT version FROM seaql_migrations'), /m0003_external_storage/);
});
