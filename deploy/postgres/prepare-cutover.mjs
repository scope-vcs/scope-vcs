// Prepare a private, reviewable credential bundle. This module performs no network requests.
import { createHash, createHmac, pbkdf2Sync, randomBytes } from 'node:crypto';
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { grants, renderPolicy, tables } from './runtime-roles.mjs';

export const serviceRoles = {
  api: 'scope_api', 'run-worker': 'scope_run_worker', cache: 'scope_cache',
  'media-api': 'scope_media_api', 'media-worker': 'scope_media_worker', maintenance: 'scope_migrator',
};
const quote = value => `'${value.replaceAll("'", "'\\''")}'`;
// Generated passwords are ASCII. PostgreSQL verifier format:
// https://doxygen.postgresql.org/scram-common_8c_source.html
export function scramVerifier(password, salt = randomBytes(16)) {
  const salted = pbkdf2Sync(password, salt, 4096, 32, 'sha256');
  const hmac = value => createHmac('sha256', salted).update(value).digest();
  return `SCRAM-SHA-256$4096:${salt.toString('base64')}$${createHash('sha256').update(hmac('Client Key')).digest('base64')}:${hmac('Server Key').toString('base64')}`;
}
function connectionScript(url, sql) {
  const variables = { PGHOST: url.hostname, PGPORT: url.port || '5432', PGDATABASE: decodeURIComponent(url.pathname.slice(1)),
    PGUSER: decodeURIComponent(url.username), PGPASSWORD: decodeURIComponent(url.password),
    PGSSLMODE: url.searchParams.get('sslmode'), PGCONNECT_TIMEOUT: '10' };
  return `#!/bin/sh\nset -eu\n${Object.entries(variables).map(([key,value]) => `export ${key}=${quote(value)}`).join('\n')}\nexec psql -X -q -v ON_ERROR_STOP=1 <<'SCOPE_CUTOVER_SQL'\n${sql}\nSCOPE_CUTOVER_SQL\n`;
}
function verifySql(role) {
  const privilegeChecks = (role === 'scope_migrator' ? [] : tables).flatMap(table => {
    const privileges = grants[role][table] ?? [];
    return ['SELECT','INSERT','UPDATE','DELETE'].map(privilege =>
      `has_table_privilege(current_user, 'public.${table}', '${privilege}') = ${privileges.includes(privilege)}`);
  });
  if (role !== 'scope_migrator') privilegeChecks.push(
    `NOT has_schema_privilege(current_user, 'public', 'CREATE')`,
    `NOT has_database_privilege(current_user, current_database(), 'TEMP')`,
    `NOT has_table_privilege(current_user, 'public.seaql_migrations', 'UPDATE')`,
  );
  return `DO $verify$ BEGIN
  IF current_user <> '${role}' OR EXISTS (SELECT 1 FROM pg_roles WHERE rolname=current_user AND (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls)) THEN
    RAISE EXCEPTION 'Unexpected credential identity or elevated role attributes';
  END IF;
  IF NOT (${privilegeChecks.length ? privilegeChecks.join('\n    AND ') : 'true'}) THEN
    RAISE EXCEPTION 'Runtime privileges do not match the reviewed policy';
  END IF;
END $verify$;
SELECT current_user AS verified_database_login;`;
}

export function prepareCutover(manifest, environment, bootstrapUrl, directory) {
  if (!['staging','production'].includes(environment)) throw new Error('Choose staging or production.');
  const url = new URL(bootstrapUrl.trim());
  if (!['postgres:','postgresql:'].includes(url.protocol) || !url.hostname.endsWith('.railway.internal') ||
      !url.username || !url.password || url.pathname.length < 2 || url.hash ||
      [...url.searchParams.keys()].some(key => key !== 'sslmode')) {
    throw new Error('A private Railway bootstrap database URL with explicit credentials is required.');
  }
  const sslmode = url.searchParams.get('sslmode');
  if (sslmode && !['disable','allow','prefer','require','verify-ca','verify-full'].includes(sslmode)) {
    throw new Error('Unsupported PostgreSQL TLS mode.');
  }
  if (!['verify-ca','verify-full'].includes(sslmode)) url.searchParams.set('sslmode','require');
  const environmentId = manifest.environments[environment].environmentId;
  const projectId = manifest.railway.projectId;
  const uuid = /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i;
  const rows = Object.entries(serviceRoles).map(([component,role]) => ({component,role,
    serviceId: component === 'maintenance' ? manifest.railway.maintenanceServiceId : manifest.services[component].id}));
  if (![projectId,environmentId,...rows.map(row=>row.serviceId)].every(value=>uuid.test(value)) ||
      new Set(rows.map(row=>row.serviceId)).size !== rows.length ||
      rows.some(row=>row.serviceId === manifest.railway.databaseServiceId) ||
      manifest.environments.staging.environmentId === manifest.environments.production.environmentId) {
    throw new Error('Cutover requires distinct explicit service and environment identities.');
  }
  mkdirSync(directory,{mode:0o700});
  const write = (file,value) => writeFileSync(join(directory,file),value,{mode:0o600,flag:'wx'});
  try {
    const credentials = rows.map(row => ({...row,password:randomBytes(32).toString('base64url')}));
    const roleSql = renderPolicy();
    if (!roleSql.endsWith('COMMIT;\n')) throw new Error('Role policy must finish its transaction explicitly.');
    const sql = roleSql.slice(0,-'COMMIT;\n'.length) + credentials.map(({role,password}) =>
      `ALTER ROLE ${role} PASSWORD '${scramVerifier(password)}';`).join('\n') + '\nCOMMIT;\n';
    write('bootstrap.sh',connectionScript(url,sql));
    for (const {component,role,serviceId,password} of credentials) {
      const connection = new URL(url);
      connection.username=role; connection.password=password;
      write(`${component}.variables.json`,`${JSON.stringify({input:{projectId,environmentId,serviceId,
        replace:false,skipDeploys:true,variables:{DATABASE_URL:connection.toString()}}})}\n`);
      write(`${component}.verify.sh`,connectionScript(connection,verifySql(role)));
      if (component === 'maintenance') {
        const fields = [connection.hostname, connection.port || '5432', decodeURIComponent(connection.pathname.slice(1)),
          role, password, connection.searchParams.get('sslmode')];
        if (fields.some(value => /[\r\n]/.test(value))) throw new Error('Connection fields cannot contain line breaks.');
        write('maintenance.connection', `${fields.join('\n')}\n`);
      }
    }
    write('plan.json',`${JSON.stringify({environment,environmentId,projectId,roles:rows},null,2)}\n`);
    return {environment,environmentId,roles:rows.length};
  } catch(error) {
    rmSync(directory,{recursive:true,force:true});
    throw error;
  }
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const [environment,directory,extra]=process.argv.slice(2);
    if (!directory || extra) throw new Error('usage: prepare-cutover.mjs staging|production NEW_PRIVATE_DIRECTORY < private-database-url');
    const manifest=JSON.parse(readFileSync(process.env.SCOPE_DEPLOYMENT_MANIFEST || '.github/deployment-services.json','utf8'));
    process.stdout.write(`${JSON.stringify(prepareCutover(manifest,environment,readFileSync(0,'utf8'),directory))}\n`);
  } catch(error) { console.error(error.message); process.exitCode=1; }
}
