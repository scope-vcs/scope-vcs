import { pathToFileURL } from 'node:url';

// Explicit inventory: migrations must update this policy before runtime cutover.
const names = (value) => value.trim().split(/\s+/).map((name) => `scope_${name}`);
export const cacheTables = names(`cache_deletion_queue cache_objects cache_orphan_uploads
  cache_references cache_uploads`);
export const mediaTables = names(`request_media_abandoned_objects request_media_attachments
  request_media_bindings request_media_cleanup_jobs request_media_derivatives
  request_media_manifest_chunks request_media_manifests request_media_orphan_cleanup_leases
  request_media_processing_jobs request_media_processing_objects request_media_retry_operations
  request_media_upload_parts`);
const repositoryTables = names(`file_changes git_compaction_jobs git_heads git_segment_references
  git_segment_uploads git_segments live_files logical_commits metadata_locks object_references
  orphan_object_jobs outbox_jobs projection_files projection_read_models push_trigger_evaluations
  repo_storage_cleanup_jobs repositories repository_first_push_tokens repository_git_push_tokens
  repository_history_entries repository_history_views repository_invites repository_invite_links
  repository_invite_emails repository_landing_files repository_members repository_workflow_catalogs repository_workflow_files visibility_change_sets
  visibility_changes workflow_revisions dependency_analyses dependency_reports dependency_analysis_jobs
  request_ref_cleanup_jobs`);
const runTables = names(`run_attempt_cache_setups run_attempt_caches run_attempt_steps run_attempts
  runs run_jobs run_logs`);
const collaborationTables = names(`request_discussion_read_states request_discussion_replies
  request_discussions request_events request_invitees request_ratings request_revisions requests
  request_claims request_attention_states request_check_evaluations request_auto_merge_intents`);
const authTables = names(`auth_identities cli_browser_logins cli_device_logins cli_exchange_grants
  cli_sessions clerk_user_deletions users`);
export const tables = [...cacheTables, ...mediaTables, ...repositoryTables, ...runTables,
  ...collaborationTables, ...authTables, 'seaql_migrations'].sort();
const crud = ['SELECT', 'INSERT', 'UPDATE', 'DELETE'];
function policy(write, read = [], lock = []) {
  return Object.fromEntries([
    ...write.map((table) => [table, crud]),
    ...read.map((table) => [table, ['SELECT']]),
    ...lock.map((table) => [table, ['SELECT', 'INSERT', 'UPDATE']]),
    ['seaql_migrations', ['SELECT']],
  ]);
}
// API owns repository/collaboration changes and media upload admission. It has no cache-store access.
// Worker also performs outbox delivery, compaction, dependency analysis and content cleanup.
export const grants = {
  scope_api: policy([...repositoryTables, ...runTables, ...collaborationTables, ...authTables, ...mediaTables]),
  scope_run_worker: { ...policy([...names(`git_compaction_jobs git_segment_references git_segment_uploads
    git_segments metadata_locks object_references orphan_object_jobs outbox_jobs projection_files
    projection_read_models push_trigger_evaluations repo_storage_cleanup_jobs workflow_revisions
    dependency_analyses dependency_reports dependency_analysis_jobs request_ref_cleanup_jobs`), ...runTables],
    names(`file_changes git_heads live_files logical_commits repositories repository_first_push_tokens
    repository_git_push_tokens repository_invites
    repository_invite_links repository_landing_files repository_members repository_workflow_catalogs repository_workflow_files
    visibility_change_sets visibility_changes requests request_revisions users request_check_evaluations`)),
    // Rebuilding a view deletes its entries through the foreign key cascade; no direct entry DELETE is needed.
    scope_repository_history_views: ['SELECT', 'INSERT', 'DELETE'],
    scope_repository_history_entries: ['SELECT', 'INSERT'],
    // Terminal check runs stop auto-merge and persist its request activity and event.
    scope_requests: ['SELECT', 'UPDATE'],
    scope_request_auto_merge_intents: ['SELECT', 'UPDATE'],
    scope_request_events: ['SELECT', 'INSERT'] },
  scope_cache: policy(cacheTables, names('runs run_jobs run_attempts')),
  scope_media_api: { ...policy(names('request_media_upload_parts request_media_abandoned_objects'),
    [...names('request_media_bindings request_media_cleanup_jobs request_media_derivatives request_media_manifest_chunks request_media_manifests'),
      ...names('repositories repository_members requests request_invitees request_discussions')],
    names('metadata_locks')), scope_request_media_attachments: ['SELECT', 'UPDATE'] },
  scope_media_worker: policy(mediaTables, names('repositories requests'), names('metadata_locks')),
};
const literal = (value) => `'${value.replaceAll("'", "''")}'`;
const identifier = (value) => `"${value.replaceAll('"', '""')}"`;

// Rendering is deliberately separate from execution. Pipe through private maintenance psql only.
export function renderPolicy({ grantsOnly = false } = {}) {
  const roles = Object.keys(grants);
  const allRoles = ['scope_migrator', ...roles];
  const sql = [`\\set ON_ERROR_STOP on`, 'BEGIN;', `SET LOCAL lock_timeout = '15s';`,
    `SET LOCAL statement_timeout = '120s';`,
    `SELECT pg_advisory_xact_lock(hashtextextended('scope:runtime-role-policy', 0));`,
    `DO $guard$ BEGIN
  IF EXISTS (SELECT 1 FROM pg_namespace WHERE nspname NOT LIKE 'pg_%'
      AND nspname NOT IN ('public', 'information_schema')) THEN
    RAISE EXCEPTION 'Role policy requires a dedicated database with only the public application schema';
  END IF;
  IF EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
      WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p', 'v', 'm', 'f')
      AND c.relname NOT IN (${tables.map(literal).join(', ')})) THEN
    RAISE EXCEPTION 'Unreviewed public relation: update the explicit role policy';
  END IF;
  IF EXISTS (SELECT 1 FROM pg_auth_members m JOIN pg_roles r ON r.oid = m.member
      WHERE r.rolname IN (${allRoles.map(literal).join(', ')})
      AND NOT (r.rolname = 'scope_migrator' AND m.roleid = 'pg_signal_backend'::regrole)) THEN
    RAISE EXCEPTION 'Scope roles must have no memberships except migrator pg_signal_backend';
  END IF;
END $guard$;`];
  for (const role of grantsOnly ? [] : allRoles) {
    sql.push(`DO $role$ BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = ${literal(role)}) THEN
    CREATE ROLE ${role} LOGIN;
  END IF;
END $role$;`,
    `ALTER ROLE ${role} NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS ${role === 'scope_migrator' ? 'INHERIT' : 'NOINHERIT'};`,
    `ALTER ROLE ${role} SET search_path = public, pg_catalog;`);
  }
  if (!grantsOnly) sql.push('GRANT pg_signal_backend TO scope_migrator;');
  sql.push(`DO $attributes$ BEGIN
  IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname IN (${roles.map(literal).join(', ')})
      AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls)) THEN
    RAISE EXCEPTION 'Unsafe runtime role attributes require administrator bootstrap';
  END IF;
END $attributes$;`,
    `SELECT format('ALTER DATABASE %I OWNER TO scope_migrator', current_database()) \\gexec`,
    `SELECT format('REVOKE ALL ON DATABASE %I FROM PUBLIC, ${roles.join(', ')}', current_database()) \\gexec`,
    `SELECT format('GRANT CONNECT ON DATABASE %I TO ${roles.join(', ')}', current_database()) \\gexec`,
    'ALTER SCHEMA public OWNER TO scope_migrator;',
    `REVOKE ALL ON SCHEMA public FROM PUBLIC, ${roles.join(', ')};`,
    `GRANT USAGE ON SCHEMA public TO ${roles.join(', ')};`,
    `REVOKE ALL ON ALL TABLES IN SCHEMA public FROM PUBLIC, ${roles.join(', ')};`,
    `SELECT format('REVOKE ALL (%s) ON TABLE public.%I FROM PUBLIC, ${roles.join(', ')}',
      string_agg(quote_ident(a.attname), ', ' ORDER BY a.attnum), c.relname)
      FROM pg_class c JOIN pg_attribute a ON a.attrelid = c.oid
      WHERE c.relnamespace = 'public'::regnamespace AND c.relkind IN ('r', 'p')
        AND a.attnum > 0 AND NOT a.attisdropped GROUP BY c.relname \\gexec`,
    `REVOKE ALL ON ALL SEQUENCES IN SCHEMA public FROM PUBLIC, ${roles.join(', ')};`,
    `SELECT format('REVOKE ALL ON ROUTINE %s FROM PUBLIC, ${roles.join(', ')}', p.oid::regprocedure)
      FROM pg_proc p WHERE p.pronamespace = 'public'::regnamespace
      AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_proc'::regclass
        AND d.objid = p.oid AND d.deptype = 'e') \\gexec`,
    `SELECT format('ALTER TABLE public.%I OWNER TO scope_migrator', relname)
      FROM pg_class WHERE relnamespace = 'public'::regnamespace AND relkind IN ('r', 'p') \\gexec`,
    `SELECT format('ALTER SEQUENCE public.%I OWNER TO scope_migrator', relname)
      FROM pg_class WHERE relnamespace = 'public'::regnamespace AND relkind = 'S' \\gexec`,
    `SELECT format('ALTER ROUTINE %s OWNER TO scope_migrator', p.oid::regprocedure)
      FROM pg_proc p WHERE p.pronamespace = 'public'::regnamespace
      AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.classid = 'pg_proc'::regclass
        AND d.objid = p.oid AND d.deptype = 'e') \\gexec`,
    `ALTER DEFAULT PRIVILEGES FOR ROLE scope_migrator REVOKE ALL ON TABLES FROM PUBLIC, ${roles.join(', ')};`,
    `ALTER DEFAULT PRIVILEGES FOR ROLE scope_migrator REVOKE ALL ON SEQUENCES FROM PUBLIC, ${roles.join(', ')};`,
    `ALTER DEFAULT PRIVILEGES FOR ROLE scope_migrator REVOKE ALL ON FUNCTIONS FROM PUBLIC, ${roles.join(', ')};`,
    `ALTER DEFAULT PRIVILEGES FOR ROLE scope_migrator IN SCHEMA public REVOKE ALL ON TABLES FROM PUBLIC, ${roles.join(', ')};`,
    `ALTER DEFAULT PRIVILEGES FOR ROLE scope_migrator IN SCHEMA public REVOKE ALL ON SEQUENCES FROM PUBLIC, ${roles.join(', ')};`,
    `ALTER DEFAULT PRIVILEGES FOR ROLE scope_migrator IN SCHEMA public REVOKE ALL ON FUNCTIONS FROM PUBLIC, ${roles.join(', ')};`);
  for (const [role, policy] of Object.entries(grants)) {
    for (const [table, privileges] of Object.entries(policy)) {
      sql.push(`GRANT ${privileges.join(', ')} ON TABLE public.${identifier(table)} TO ${role};`);
    }
  }
  sql.push('GRANT USAGE ON SEQUENCE public.scope_run_creation_sequence TO scope_api, scope_run_worker;', 'COMMIT;');
  return `${sql.join('\n')}\n`;
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  if (process.argv.slice(2).some((arg) => arg !== '--grants-only')) throw new Error('Usage: runtime-roles.mjs [--grants-only]');
  process.stdout.write(renderPolicy({ grantsOnly: process.argv.includes('--grants-only') }));
}
