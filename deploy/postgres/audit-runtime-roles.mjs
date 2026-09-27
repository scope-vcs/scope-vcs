import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { grants, tables } from './runtime-roles.mjs';

const runtimeRoles = Object.keys(grants);
const roles = ['scope_migrator', ...runtimeRoles];
const sqlString = value => `'${value.replaceAll("'", "''")}'`;
const migrationSources = JSON.parse(readFileSync(new URL('../../crates/scope-postgres/src/migrations/sources.lock.json', import.meta.url)));
export const candidateMigrationVersion = Object.keys(migrationSources)
  .filter(name => /^m\d+_.+\.rs$/.test(name)).sort().at(-1)?.slice(0, -3);
if (!candidateMigrationVersion) throw new Error('Cannot establish candidate migration version for role audit');

// Run as scope_migrator through the private maintenance connection. Pending
// migrations can change grants, so compare exact grants only after the candidate
// ledger version is present. Ownership and role safety always remain required.
export function renderRuntimeRoleAudit({ exactPolicy = true } = {}) {
  const policy = sqlString(JSON.stringify(grants));
  const expectedTables = tables.map(sqlString).join(', ');
  const roleNames = roles.map(sqlString).join(', ');
  const runtimeNames = runtimeRoles.map(sqlString).join(', ');
  return `\\set ON_ERROR_STOP on
BEGIN READ ONLY;
SET LOCAL statement_timeout = '60s';
DO $audit$
DECLARE
  policy jsonb := ${policy}::jsonb;
  role_name text;
  relation record;
  permission text;
  expected boolean;
  actual boolean;
  exact_policy boolean;
BEGIN
  IF current_user <> 'scope_migrator' THEN
    RAISE EXCEPTION 'Production role audit requires scope_migrator';
  END IF;
  IF to_regclass('public.seaql_migrations') IS NULL THEN
    RAISE EXCEPTION 'Production migration ledger is missing';
  END IF;
  exact_policy := ${exactPolicy ? `EXISTS (SELECT 1 FROM public.seaql_migrations WHERE version = ${sqlString(candidateMigrationVersion)})` : 'false'};
  IF (SELECT pg_get_userbyid(datdba) FROM pg_database WHERE datname = current_database()) <> 'scope_migrator'
    OR (SELECT pg_get_userbyid(nspowner) FROM pg_namespace WHERE nspname = 'public') <> 'scope_migrator' THEN
    RAISE EXCEPTION 'Production database or public schema is not owned by scope_migrator';
  END IF;
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname IN (${roleNames})
      AND (NOT rolcanlogin OR rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls))
    OR (SELECT count(*) FROM pg_roles WHERE rolname IN (${roleNames})) <> ${roles.length} THEN
    RAISE EXCEPTION 'Production database roles are missing or elevated';
  END IF;
  IF EXISTS (SELECT 1 FROM pg_auth_members m JOIN pg_roles r ON r.oid = m.member
      WHERE r.rolname IN (${roleNames})
      AND NOT (r.rolname = 'scope_migrator' AND m.roleid = 'pg_signal_backend'::regrole))
    OR NOT pg_has_role('scope_migrator', 'pg_signal_backend', 'MEMBER') THEN
    RAISE EXCEPTION 'Production role memberships differ from policy';
  END IF;
  IF EXISTS (SELECT 1 FROM pg_auth_members m JOIN pg_roles r ON r.oid = m.roleid
      WHERE r.rolname IN (${roleNames})) THEN
    RAISE EXCEPTION 'Production role is held by another account';
  END IF;
  IF EXISTS (SELECT 1 FROM pg_namespace WHERE nspname NOT LIKE 'pg_%'
      AND nspname NOT IN ('public', 'information_schema')) THEN
    RAISE EXCEPTION 'Unexpected production application schema';
  END IF;
  IF exact_policy AND EXISTS (SELECT 1 FROM unnest(ARRAY[${expectedTables}]) AS expected(relname)
      WHERE to_regclass('public.' || quote_ident(expected.relname)) IS NULL) THEN
    RAISE EXCEPTION 'Expected production relation is missing';
  END IF;
  IF exact_policy AND EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
      WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
      AND c.relname NOT IN (${expectedTables})) THEN
    RAISE EXCEPTION 'Unreviewed production relation';
  END IF;
  IF EXISTS (SELECT 1 FROM pg_class WHERE relnamespace = 'public'::regnamespace
      AND relkind IN ('r', 'p', 'v', 'm', 'f', 'S')
      AND pg_get_userbyid(relowner) <> 'scope_migrator')
    OR EXISTS (SELECT 1 FROM pg_proc p WHERE p.pronamespace = 'public'::regnamespace
      AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_proc'::regclass
        AND d.objid = p.oid AND d.deptype = 'e')
      AND pg_get_userbyid(p.proowner) <> 'scope_migrator') THEN
    RAISE EXCEPTION 'Production application objects are not owned by scope_migrator';
  END IF;

  FOREACH role_name IN ARRAY ARRAY[${runtimeNames}] LOOP
    IF NOT has_database_privilege(role_name, current_database(), 'CONNECT')
      OR has_database_privilege(role_name, current_database(), 'CREATE')
      OR has_database_privilege(role_name, current_database(), 'TEMP')
      OR NOT has_schema_privilege(role_name, 'public', 'USAGE')
      OR has_schema_privilege(role_name, 'public', 'CREATE') THEN
      RAISE EXCEPTION 'Production database/schema privileges differ for %', role_name;
    END IF;
    -- The ledger stays read-only in every mode: a writable ledger lets a runtime
    -- role forge the candidate version that unlocks the exact comparison.
    IF NOT has_table_privilege(role_name, 'public.seaql_migrations', 'SELECT')
      OR has_any_column_privilege(role_name, 'public.seaql_migrations', 'INSERT')
      OR has_any_column_privilege(role_name, 'public.seaql_migrations', 'UPDATE')
      OR has_any_column_privilege(role_name, 'public.seaql_migrations', 'REFERENCES')
      OR has_table_privilege(role_name, 'public.seaql_migrations', 'DELETE')
      OR has_table_privilege(role_name, 'public.seaql_migrations', 'TRUNCATE')
      OR has_table_privilege(role_name, 'public.seaql_migrations', 'TRIGGER') THEN
      RAISE EXCEPTION 'Production migration-ledger privileges differ for %', role_name;
    END IF;
    IF NOT exact_policy THEN
      CONTINUE;
    END IF;
    FOR relation IN SELECT c.oid, c.relname FROM pg_class c
      WHERE c.relnamespace = 'public'::regnamespace AND c.relkind IN ('r', 'p') LOOP
      FOREACH permission IN ARRAY ARRAY['SELECT', 'INSERT', 'UPDATE', 'DELETE', 'TRUNCATE', 'REFERENCES', 'TRIGGER'] LOOP
        expected := COALESCE((policy -> role_name -> relation.relname) ? permission, false);
        actual := has_table_privilege(role_name, relation.oid, permission);
        IF actual IS DISTINCT FROM expected THEN
          RAISE EXCEPTION 'Production table privilege differs: % %.%', role_name, relation.relname, permission;
        END IF;
        IF permission IN ('SELECT', 'INSERT', 'UPDATE', 'REFERENCES') AND EXISTS (
          SELECT 1 FROM pg_attribute a WHERE a.attrelid = relation.oid AND a.attnum > 0 AND NOT a.attisdropped
            AND has_column_privilege(role_name, relation.oid, a.attnum, permission) IS DISTINCT FROM expected
        ) THEN
          RAISE EXCEPTION 'Production column privilege differs: % %.%', role_name, relation.relname, permission;
        END IF;
      END LOOP;
    END LOOP;
    FOR relation IN SELECT c.oid, c.relname FROM pg_class c
      WHERE c.relnamespace = 'public'::regnamespace AND c.relkind = 'S' LOOP
      FOREACH permission IN ARRAY ARRAY['USAGE', 'SELECT', 'UPDATE'] LOOP
        expected := relation.relname = 'scope_run_creation_sequence'
          AND role_name IN ('scope_api', 'scope_run_worker') AND permission = 'USAGE';
        IF has_sequence_privilege(role_name, relation.oid, permission) IS DISTINCT FROM expected THEN
          RAISE EXCEPTION 'Production sequence privilege differs: % %.%', role_name, relation.relname, permission;
        END IF;
      END LOOP;
    END LOOP;
    FOR relation IN SELECT p.oid, p.proname FROM pg_proc p
      WHERE p.pronamespace = 'public'::regnamespace
        AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_proc'::regclass
          AND d.objid = p.oid AND d.deptype = 'e') LOOP
      IF has_function_privilege(role_name, relation.oid, 'EXECUTE') THEN
        RAISE EXCEPTION 'Production routine privilege differs: % %', role_name, relation.proname;
      END IF;
    END LOOP;
  END LOOP;
END $audit$;
COMMIT;
`;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.stdout.write(renderRuntimeRoleAudit());
}
