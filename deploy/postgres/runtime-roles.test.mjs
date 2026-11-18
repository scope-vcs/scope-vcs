import assert from 'node:assert/strict';
import { execFileSync, spawn, spawnSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { grants, renderPolicy, retiredRoles, tables } from './runtime-roles.mjs';
import { candidateMigrationVersion, renderRuntimeRoleAudit } from './audit-runtime-roles.mjs';
import { localClusterSkip, pgBin } from './test-cluster.mjs';

const repositoryRoot = fileURLToPath(new URL('../../', import.meta.url));
const migrationTest = 'db::migration_tests::runtime_role_cluster::runtime_role_cluster_receives_the_production_migrations';

function applyProductionMigrations(dir, role) {
  const migration = spawnSync('cargo', ['test', '-p', 'scope-postgres', '--lib',
    migrationTest, '--', '--exact', '--nocapture'], {
    cwd: repositoryRoot,
    env: {
      ...process.env,
      SCOPE_RUNTIME_ROLE_MIGRATION_URL: `postgres://${role}@localhost/postgres?host=${encodeURIComponent(dir)}`,
    },
    encoding: 'utf8', timeout: 600_000, maxBuffer: 10 * 1024 * 1024,
  });
  assert.equal(migration.status, 0, `${migration.stdout}\n${migration.stderr}`);
  assert.match(migration.stdout, new RegExp(`test ${migrationTest} \\.\\.\\. ok`));
}

test('retired roles lose access under every policy and only the administrator policy drops them', () => {
  assert.deepEqual(retiredRoles.filter((role) => role in grants), []);
  assert.doesNotMatch(renderPolicy({ mode: 'grants' }), /DROP (OWNED|ROLE)/);
  for (const mode of ['full', 'roles']) {
    assert.match(renderPolicy({ mode }), /REVOKE ALL ON DATABASE[\s\S]*pg_terminate_backend[\s\S]*DROP OWNED BY[\s\S]*DROP ROLE/);
  }
});

test('runtime roles enforce service boundaries on PostgreSQL', { skip: localClusterSkip }, async () => {
  const dir = mkdtempSync(join(tmpdir(), 'scope-role-test-'));
  const data = join(dir, 'data');
  let started = false;
  function query(sql, role = 'postgres', succeeds = true) {
    const result = spawnSync(join(pgBin, 'psql'), ['-X', '-qAt', '-v', 'ON_ERROR_STOP=1',
      '-h', dir, '-U', role, '-d', 'postgres'], { input: sql, encoding: 'utf8' });
    if (succeeds) assert.equal(result.status, 0, result.stderr);
    else {
      assert.notEqual(result.status, 0, sql);
      assert.match(result.stderr, /permission denied|must be owner|must be superuser|not permitted/);
    }
    return result;
  }
  try {
    execFileSync(join(pgBin, 'initdb'), ['-D', data, '-U', 'postgres', '--auth=trust', '--no-locale'], { stdio: 'pipe' });
    execFileSync(join(pgBin, 'pg_ctl'), ['-D', data, '-l', join(dir, 'log'), '-o', `-k ${dir} -c listen_addresses=''`, '-w', 'start'], { stdio: 'pipe' });
    started = true;
    applyProductionMigrations(dir, 'postgres');
    query(`DELETE FROM seaql_migrations WHERE version = '${candidateMigrationVersion}';`);
    const actual = query(`SELECT c.relname FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
      WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p', 'v', 'm', 'f') ORDER BY c.relname`).stdout.trim().split('\n');
    assert.deepEqual(actual, tables);
    query(renderPolicy());
    query('GRANT SELECT (id) ON scope_cli_sessions TO scope_media_worker;');
    query(renderPolicy());
    query('SELECT id FROM scope_cli_sessions;', 'scope_media_worker', false);
    query(renderPolicy({ mode: 'grants' }), 'scope_migrator');
    query(renderRuntimeRoleAudit(), 'scope_migrator');
    query('GRANT SELECT (id) ON scope_cli_sessions TO scope_media_worker;');
    query(renderRuntimeRoleAudit(), 'scope_migrator');
    query('CREATE TABLE old_pending_relation(id int);', 'scope_migrator');
    query(renderRuntimeRoleAudit(), 'scope_migrator');
    query(`INSERT INTO seaql_migrations(version, applied_at) VALUES ('${candidateMigrationVersion}', 0);`);
    const auditFails = (pattern, audit = renderRuntimeRoleAudit()) => {
      const result = spawnSync(join(pgBin, 'psql'), ['-X', '-qAt', '-v', 'ON_ERROR_STOP=1',
        '-h', dir, '-U', 'scope_migrator', '-d', 'postgres'], { input: audit, encoding: 'utf8' });
      assert.notEqual(result.status, 0);
      assert.match(result.stderr, pattern);
    };
    auditFails(/Unreviewed production relation/);
    query('DROP TABLE old_pending_relation;', 'scope_migrator');
    auditFails(/Production column privilege differs/);
    query(renderPolicy({ mode: 'grants' }), 'scope_migrator');
    query(renderRuntimeRoleAudit(), 'scope_migrator');
    query('ALTER TABLE scope_cli_sessions OWNER TO postgres;');
    auditFails(/not owned by scope_migrator/);
    query('ALTER TABLE scope_cli_sessions OWNER TO scope_migrator;');
    query('GRANT scope_media_worker TO postgres;');
    auditFails(/held by another account/);
    query('REVOKE scope_media_worker FROM postgres;');
    query('GRANT INSERT ON seaql_migrations TO scope_media_worker;');
    auditFails(/migration-ledger privileges/, renderRuntimeRoleAudit({ exactPolicy: false }));
    query('REVOKE INSERT ON seaql_migrations FROM scope_media_worker;');
    query('ALTER TABLE scope_cli_sessions RENAME TO scope_cli_sessions_retired;', 'scope_migrator');
    auditFails(/relation is missing/);
    query('ALTER TABLE scope_cli_sessions_retired RENAME TO scope_cli_sessions;', 'scope_migrator');
    query(renderRuntimeRoleAudit(), 'scope_migrator');
    const roleCount = () => query("SELECT count(*) FROM pg_roles WHERE rolname = 'scope_run_worker'").stdout.trim();
    query(`CREATE ROLE scope_run_worker LOGIN;
      GRANT CONNECT ON DATABASE postgres TO scope_run_worker;
      GRANT USAGE ON SCHEMA public TO scope_run_worker;
      GRANT SELECT, UPDATE ON scope_repositories TO scope_run_worker;
      GRANT SELECT (id) ON scope_cli_sessions TO scope_run_worker;`);
    query('SELECT count(*) FROM scope_repositories;', 'scope_run_worker');
    auditFails(/Retired production role keeps access/);
    const retiredSession = spawn(join(pgBin, 'psql'), ['-X', '-qAt', '-h', dir, '-U', 'scope_run_worker', '-d', 'postgres'], { stdio: ['pipe', 'pipe', 'pipe'] });
    retiredSession.stderr.resume();
    const retiredSessionClosed = new Promise((resolve) => retiredSession.once('close', resolve));
    retiredSession.stdin.end('SELECT pg_backend_pid(); SELECT pg_sleep(30);');
    await new Promise((resolve, reject) => {
      retiredSession.stdout.once('data', resolve);
      retiredSession.once('error', reject);
    });
    query(renderPolicy({ mode: 'grants' }), 'scope_migrator');
    await retiredSessionClosed;
    query(renderRuntimeRoleAudit(), 'scope_migrator');
    query('SELECT 1;', 'scope_run_worker', false);
    assert.equal(roleCount(), '1');
    query(renderPolicy());
    assert.equal(roleCount(), '0');
    query(renderRuntimeRoleAudit(), 'scope_migrator');
    const sleeper = spawn(join(pgBin, 'psql'), ['-X', '-qAt', '-h', dir, '-U', 'scope_media_worker', '-d', 'postgres'], { stdio: ['pipe', 'pipe', 'pipe'] });
    sleeper.stderr.resume();
    sleeper.stdin.end('SELECT pg_backend_pid(); SELECT pg_sleep(30);');
    const pid = await new Promise((resolve, reject) => {
      sleeper.stdout.once('data', (data) => resolve(Number(data.toString().trim())));
      sleeper.once('error', reject);
    });
    assert(Number.isInteger(pid));
    assert.equal(query(`SELECT pg_terminate_backend(${pid});`, 'scope_migrator').stdout.trim(), 't');
    for (const [role, policy] of Object.entries(grants)) {
      assert.equal(query(`SELECT rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls FROM pg_roles WHERE rolname = '${role}'`).stdout.trim(), 'f');
      assert.equal(query(`SELECT count(*) FROM pg_class WHERE relowner = '${role}'::regrole`).stdout.trim(), '0');
      const effective = new Map(tables.map((table) => [table, []]));
      const rows = query(`SELECT t, p FROM unnest(ARRAY[${tables.map((table) => `'${table}'`).join(',')}]) t
        CROSS JOIN unnest(ARRAY['SELECT','INSERT','UPDATE','DELETE','TRUNCATE','REFERENCES','TRIGGER']) WITH ORDINALITY permissions(p, position)
        WHERE has_table_privilege('${role}', 'public.' || t, p) ORDER BY t, position`).stdout.trim().split('\n');
      for (const row of rows) {
        const [table, permission] = row.split('|');
        effective.get(table).push(permission);
      }
      for (const table of tables) assert.deepEqual(effective.get(table), policy[table] ?? [], `${role}: ${table}`);
      query('SELECT version FROM seaql_migrations;', role);
      query('CREATE TABLE public.forbidden (id int);', role, false);
      query('CREATE TEMP TABLE forbidden (id int);', role, false);
      query('CREATE SCHEMA forbidden;', role, false);
      query('ALTER TABLE scope_repositories ADD COLUMN forbidden int;', role, false);
      query('SET ROLE scope_migrator;', role, false);
      query('UPDATE seaql_migrations SET version = version;', role, false);
      query('CREATE ROLE forbidden;', role, false);
    }
    query(`SELECT * FROM scope_request_revision_context_receipts;
      INSERT INTO scope_request_revision_context_receipts SELECT * FROM scope_request_revision_context_receipts WHERE false;
      UPDATE scope_request_revision_context_receipts SET payload_digest = payload_digest;
      DELETE FROM scope_request_revision_context_receipts WHERE false;`, 'scope_api');
    for (const role of Object.keys(grants).filter((role) => role !== 'scope_api')) {
      query('SELECT * FROM scope_request_revision_context_receipts;', role, false);
      query('INSERT INTO scope_request_revision_context_receipts SELECT * FROM scope_request_revision_context_receipts WHERE false;', role, false);
    }
    const roleUrl = (role) => `postgres://${role}@localhost/postgres?host=${encodeURIComponent(dir)}`;
    function runRoleTest(name, env) {
      const result = spawnSync('cargo', ['test', '-p', 'scope-postgres', '--lib', name, '--', '--exact', '--nocapture'], {
        cwd: repositoryRoot,
        env: { ...process.env, ...env },
        encoding: 'utf8', timeout: 600_000, maxBuffer: 10 * 1024 * 1024,
      });
      assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`);
      assert.match(result.stdout, new RegExp(`test ${name} \\.\\.\\. ok`));
    }
    runRoleTest('db::request_media::tests::media_api_role::media_api_role_authorizes_stores_and_serves_an_upload', {
      SCOPE_MEDIA_ROLE_TEST_ADMIN_URL: roleUrl('postgres'),
      SCOPE_MEDIA_ROLE_TEST_MEDIA_API_URL: roleUrl('scope_media_api'),
    });
    query("SELECT 1 AS present FROM scope_request_discussions WHERE id = 'missing' AND request_id = 'missing';", 'scope_media_api');
    query('SELECT * FROM scope_auth_identities;', 'scope_media_api', false);
    query("INSERT INTO scope_metadata_locks(key) VALUES ('role-canary') ON CONFLICT DO NOTHING; SELECT * FROM scope_metadata_locks FOR SHARE;", 'scope_media_worker');
    const restoredSchema = execFileSync(join(pgBin, 'pg_dump'), ['-h', dir, '-U', 'postgres', '-d', 'postgres', '--schema-only', '--no-owner', '--no-acl'], { encoding: 'utf8' });
    query(`DROP SCHEMA public CASCADE; CREATE SCHEMA public; ${restoredSchema}`, 'scope_migrator');
    query('SELECT * FROM public.scope_request_media_attachments;', 'scope_media_worker', false);
    query(renderPolicy({ mode: 'grants' }), 'scope_migrator');
    query('SELECT * FROM scope_request_media_attachments;', 'scope_media_worker');
    query('SELECT * FROM scope_cli_sessions;', 'scope_media_worker', false);
    query('CREATE TABLE future_table(id int); CREATE FUNCTION future_function() RETURNS int LANGUAGE sql AS $$ SELECT 1 $$;', 'scope_migrator');
    for (const role of Object.keys(grants)) {
      query('SELECT * FROM future_table;', role, false);
      query('SELECT future_function();', role, false);
    }
    const rejected = spawnSync(join(pgBin, 'psql'), ['-X', '-qAt', '-h', dir, '-U', 'postgres', '-d', 'postgres'], { input: renderPolicy(), encoding: 'utf8' });
    assert.notEqual(rejected.status, 0);
    assert.match(rejected.stderr, /Unreviewed public relation/);
    query('DROP TABLE future_table; DROP FUNCTION future_function();', 'scope_migrator');
    query('CREATE ROLE unsafe_parent; GRANT unsafe_parent TO scope_media_worker;');
    const membership = spawnSync(join(pgBin, 'psql'), ['-X', '-qAt', '-h', dir, '-U', 'postgres', '-d', 'postgres'], { input: renderPolicy(), encoding: 'utf8' });
    assert.notEqual(membership.status, 0);
    assert.match(membership.stderr, /must have no memberships/);
  } finally {
    if (started) execFileSync(join(pgBin, 'pg_ctl'), ['-D', data, '-m', 'immediate', '-w', 'stop'], { stdio: 'pipe' });
    rmSync(dir, { recursive: true, force: true });
  }
});

test('roles-only bootstrap lets the migration login build and grant an empty database', { skip: localClusterSkip }, async () => {
  const dir = mkdtempSync(join(tmpdir(), 'scope-role-bootstrap-'));
  const data = join(dir, 'data');
  let started = false;
  function query(sql, role = 'postgres', succeeds = true) {
    const result = spawnSync(join(pgBin, 'psql'), ['-X', '-qAt', '-v', 'ON_ERROR_STOP=1',
      '-h', dir, '-U', role, '-d', 'postgres'], { input: sql, encoding: 'utf8' });
    if (succeeds) assert.equal(result.status, 0, result.stderr);
    else assert.notEqual(result.status, 0, sql);
    return result;
  }
  try {
    execFileSync(join(pgBin, 'initdb'), ['-D', data, '-U', 'postgres', '--auth=trust', '--no-locale'], { stdio: 'pipe' });
    execFileSync(join(pgBin, 'pg_ctl'), ['-D', data, '-l', join(dir, 'log'), '-o', `-k ${dir} -c listen_addresses=''`, '-w', 'start'], { stdio: 'pipe' });
    started = true;
    assert.doesNotMatch(renderPolicy({ mode: 'roles' }), /^GRANT [A-Z, ]+ ON (TABLE|SEQUENCE) public\./m);
    query(renderPolicy({ mode: 'roles' }));
    query(renderPolicy({ mode: 'roles' }));
    query('CREATE TABLE early(id int);', 'scope_api', false);
    applyProductionMigrations(dir, 'scope_migrator');
    query('SELECT * FROM scope_request_media_attachments;', 'scope_media_worker', false);
    query(renderPolicy({ mode: 'grants' }), 'scope_migrator');
    query('SELECT * FROM scope_request_media_attachments;', 'scope_media_worker');
    query('SELECT * FROM scope_cli_sessions;', 'scope_media_worker', false);
    query(renderRuntimeRoleAudit(), 'scope_migrator');
  } finally {
    if (started) execFileSync(join(pgBin, 'pg_ctl'), ['-D', data, '-m', 'immediate', '-w', 'stop'], { stdio: 'pipe' });
    rmSync(dir, { recursive: true, force: true });
  }
});
