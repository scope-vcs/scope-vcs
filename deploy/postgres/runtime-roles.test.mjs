import assert from 'node:assert/strict';
import { execFileSync, spawn, spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { grants, renderPolicy, tables } from './runtime-roles.mjs';

// Always creates its own disposable cluster. Never reads a DATABASE_URL or uses a live server.
const pgBin = process.env.SCOPE_TEST_POSTGRES_BIN ?? '/usr/lib/postgresql/16/bin';
test('runtime roles enforce service boundaries on PostgreSQL', async () => {
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
    const baseline = readFileSync(new URL('../../crates/scope-postgres/src/migrations/current_schema.sql', import.meta.url), 'utf8');
    query(baseline);
    // Apply the actual raw SQL of the migrations which introduce the additional tables.
    for (const filename of ['m0044_request_attention.rs', 'm0045_dependency_analysis.rs', 'm0053_request_ref_cleanup.rs']) {
      const source = readFileSync(new URL(`../../crates/scope-postgres/src/migrations/${filename}`, import.meta.url), 'utf8');
      query(source.match(/r#"([\s\S]*?)"#/)[1]);
    }
    query('CREATE TABLE seaql_migrations (version text PRIMARY KEY);');
    const actual = query("SELECT tablename FROM pg_tables WHERE schemaname = 'public' ORDER BY tablename").stdout.trim().split('\n');
    assert.deepEqual(actual, tables);
    query(renderPolicy());
    query('GRANT SELECT (id) ON scope_cli_sessions TO scope_cache;');
    query(renderPolicy()); // Repeat bootstrap removes stale column grants too.
    query('SELECT id FROM scope_cli_sessions;', 'scope_cache', false);
    query(renderPolicy({ grantsOnly: true }), 'scope_migrator');
    const sleeper = spawn(join(pgBin, 'psql'), ['-X', '-qAt', '-h', dir, '-U', 'scope_cache', '-d', 'postgres'], { stdio: ['pipe', 'pipe', 'pipe'] });
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
      // Compare effective permissions, including PUBLIC and inherited grants, for every service/table pair.
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
    query('SELECT * FROM scope_runs; SELECT * FROM scope_run_jobs; SELECT * FROM scope_run_attempts;', 'scope_cache');
    query('UPDATE scope_runs SET id = id;', 'scope_cache', false);
    query('SELECT * FROM scope_cli_sessions;', 'scope_run_worker', false);
    query('UPDATE scope_repository_members SET repo_id = repo_id;', 'scope_run_worker', false);
    query('UPDATE scope_repositories SET owner_user_id = owner_user_id;', 'scope_run_worker', false);
    query("SELECT 1 AS present FROM scope_request_discussions WHERE id = 'missing' AND request_id = 'missing';", 'scope_media_api');
    query('SELECT * FROM scope_cache_objects;', 'scope_api', false);
    query('SELECT * FROM scope_runs;', 'scope_media_worker', false);
    query('SELECT * FROM scope_auth_identities;', 'scope_media_api', false);
    query("INSERT INTO scope_metadata_locks(key) VALUES ('role-canary') ON CONFLICT DO NOTHING; SELECT * FROM scope_metadata_locks FOR SHARE;", 'scope_media_worker');
    const restoredSchema = execFileSync(join(pgBin, 'pg_dump'), ['-h', dir, '-U', 'postgres', '-d', 'postgres', '--schema-only', '--no-owner', '--no-acl'], { encoding: 'utf8' });
    query(`DROP SCHEMA public CASCADE; CREATE SCHEMA public; ${restoredSchema}`, 'scope_migrator');
    query('SELECT * FROM public.scope_runs;', 'scope_cache', false);
    query(renderPolicy({ grantsOnly: true }), 'scope_migrator');
    query('SELECT * FROM scope_runs;', 'scope_cache');
    query('SELECT * FROM scope_cli_sessions;', 'scope_cache', false);
    query('CREATE TABLE future_table(id int); CREATE FUNCTION future_function() RETURNS int LANGUAGE sql AS $$ SELECT 1 $$;', 'scope_migrator');
    for (const role of Object.keys(grants)) {
      query('SELECT * FROM future_table;', role, false);
      query('SELECT future_function();', role, false);
    }
    const rejected = spawnSync(join(pgBin, 'psql'), ['-X', '-qAt', '-h', dir, '-U', 'postgres', '-d', 'postgres'], { input: renderPolicy(), encoding: 'utf8' });
    assert.notEqual(rejected.status, 0);
    assert.match(rejected.stderr, /Unreviewed public relation/);
    query('DROP TABLE future_table; DROP FUNCTION future_function();', 'scope_migrator');
    query('CREATE ROLE unsafe_parent; GRANT unsafe_parent TO scope_cache;');
    const membership = spawnSync(join(pgBin, 'psql'), ['-X', '-qAt', '-h', dir, '-U', 'postgres', '-d', 'postgres'], { input: renderPolicy(), encoding: 'utf8' });
    assert.notEqual(membership.status, 0);
    assert.match(membership.stderr, /must have no memberships/);
  } finally {
    if (started) execFileSync(join(pgBin, 'pg_ctl'), ['-D', data, '-m', 'immediate', '-w', 'stop'], { stdio: 'pipe' });
    rmSync(dir, { recursive: true, force: true });
  }
});
