import { generateKeyPairSync, randomBytes } from 'node:crypto';
import { renderPolicy } from '../../deploy/postgres/runtime-roles.mjs';

const PREVIEW_NAME = /^pr-[1-9][0-9]{0,8}$/;
const UUID = /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i;
const ROLE_PASSWORD = /^[a-f0-9]{64}$/;
const POSTGRES = 'scope-postgres';
const ADMIN_DATABASE_URL = `\${{${POSTGRES}.DATABASE_URL}}`;
const PUBLIC_COMPONENTS = ['api', 'web', 'cache', 'git-router', 'media-api'];
export const RUNTIME_ROLES = Object.freeze({
  api: 'scope_api',
  'run-worker': 'scope_run_worker',
  cache: 'scope_cache',
  'media-api': 'scope_media_api',
  'media-worker': 'scope_media_worker',
});

export function previewEnvironmentName(pullRequest) {
  const name = `pr-${pullRequest}`;
  if (!PREVIEW_NAME.test(name)) throw new Error('A positive pull request number is required.');
  return name;
}

export function releaseEnvironmentIds(manifest) {
  const production = manifest?.environments?.production?.environmentId;
  const staging = manifest?.environments?.staging?.environmentId;
  if (!UUID.test(production ?? '') || !UUID.test(staging ?? '') || production === staging) {
    throw new Error('Preview environments require distinct production and staging environment IDs.');
  }
  return { production, staging };
}

export function assertPreviewEnvironment(manifest, environment, name) {
  const { production, staging } = releaseEnvironmentIds(manifest);
  if (!PREVIEW_NAME.test(name) || environment?.name !== name || environment.isEphemeral !== true ||
      !UUID.test(environment.id ?? '') || environment.id === production || environment.id === staging) {
    throw new Error(`Railway environment ${name} is not an ephemeral preview environment.`);
  }
  return environment.id;
}

export function serviceIds(manifest) {
  const ids = Object.fromEntries(Object.entries(manifest.services).map(([component, service]) => [component, service.id]));
  ids.postgres = manifest.railway.databaseServiceId;
  ids.maintenance = manifest.railway.maintenanceServiceId;
  for (const [component, id] of Object.entries(ids)) {
    if (!UUID.test(id ?? '')) throw new Error(`Preview environments require the ${component} service ID.`);
  }
  return ids;
}

export function previewDomains(manifest, config) {
  const ids = serviceIds(manifest);
  return Object.fromEntries(PUBLIC_COMPONENTS.map((component) => {
    const domains = Object.keys(config?.services?.[ids[component]]?.networking?.serviceDomains ?? {});
    if (domains.length !== 1 || !domains[0].endsWith('.up.railway.app')) {
      throw new Error(`Preview ${component} must have exactly one Railway domain.`);
    }
    return [component, domains[0]];
  }));
}

function roleDatabaseUrl(role, password) {
  if (!ROLE_PASSWORD.test(password)) throw new Error('Role passwords must be 64 hexadecimal characters.');
  return `postgresql://${role}:${password}@\${{${POSTGRES}.RAILWAY_PRIVATE_DOMAIN}}:\${{${POSTGRES}.PGPORT}}/\${{${POSTGRES}.PGDATABASE}}`;
}

function signingKeyPair() {
  const { privateKey, publicKey } = generateKeyPairSync('ed25519', {
    privateKeyEncoding: { type: 'pkcs8', format: 'pem' },
    publicKeyEncoding: { type: 'spki', format: 'pem' },
  });
  return { privateKey, publicKey };
}

export function generatePreviewSecrets() {
  const password = () => randomBytes(32).toString('hex');
  return {
    postgresPassword: password(),
    rolePasswords: Object.fromEntries(Object.keys(RUNTIME_ROLES).map((component) => [component, password()])),
    objectEncryptionKey: randomBytes(32).toString('base64'),
    mediaEncryptionKey: randomBytes(32).toString('base64'),
    cacheGrant: signingKeyPair(),
    mediaGrant: signingKeyPair(),
    operatorToken: password(),
  };
}

export function clerkDevelopmentInstance({ publishableKey, secretKey }) {
  const encoded = /^pk_test_([A-Za-z0-9_-]+={0,2})$/.exec(publishableKey ?? '')?.[1];
  const host = encoded ? Buffer.from(encoded, 'base64url').toString('utf8').replace(/\$$/, '') : '';
  if (!/^[a-z0-9-]+(?:\.[a-z0-9-]+)*\.clerk\.accounts\.dev$/.test(host) || !/^sk_test_[A-Za-z0-9]+$/.test(secretKey ?? '')) {
    throw new Error('Preview environments require a Clerk development publishable and secret key pair.');
  }
  return { publishableKey, secretKey, issuer: `https://${host}` };
}

export function previewVariables({ manifest, domains, current, secrets, clerk }) {
  const { publishableKey, secretKey, issuer } = clerkDevelopmentInstance(clerk ?? {});
  const ids = serviceIds(manifest);
  const url = (component) => `https://${domains[component]}`;
  const variables = Object.fromEntries(Object.values(ids).map((id) => [id, {}]));
  const set = (component, values) => Object.assign(variables[ids[component]], values);
  const has = (component, name) => Boolean(current[ids[component]]?.[name]);
  const group = (members, values) => {
    if (members.some(([component, name]) => !has(component, name))) {
      for (const [component, name] of members) set(component, { [name]: values[name] });
    }
  };

  set('postgres', {
    PGPASSWORD: '${{POSTGRES_PASSWORD}}',
    DATABASE_URL: 'postgresql://${{PGUSER}}:${{POSTGRES_PASSWORD}}@${{RAILWAY_PRIVATE_DOMAIN}}:${{PGPORT}}/${{PGDATABASE}}',
  });
  if (!has('postgres', 'POSTGRES_PASSWORD')) set('postgres', { POSTGRES_PASSWORD: secrets.postgresPassword });
  if (!has('maintenance', 'DATABASE_URL')) set('maintenance', { DATABASE_URL: ADMIN_DATABASE_URL });
  for (const [component, role] of Object.entries(RUNTIME_ROLES)) {
    if (!has(component, 'DATABASE_URL')) set(component, { DATABASE_URL: roleDatabaseUrl(role, secrets.rolePasswords[component]) });
  }

  const blobs = (name) => `\${{scope-blobs.${name}}}`;
  set('api', {
    CLERK_ISSUER: issuer,
    CLERK_JWKS_URL: `${issuer}/.well-known/jwks.json`,
    CLERK_AUTHORIZED_PARTIES: url('web'),
    SCOPE_API_PUBLIC_URL: url('api'),
    SCOPE_APP_ORIGIN: url('web'),
    SCOPE_CACHE_URL: url('cache'),
    SCOPE_GIT_PUBLIC_URL: url('git-router'),
    SCOPE_BUCKET_SECRET_ACCESS_KEY: blobs('SECRET_ACCESS_KEY'),
  });
  set('web', {
    CLERK_SECRET_KEY: secretKey,
    VITE_CLERK_PUBLISHABLE_KEY: publishableKey,
    SCOPE_API_HOST: domains.api,
    SCOPE_API_PUBLIC_URL: url('api'),
  });
  set('run-worker', { SCOPE_PUBLIC_API_URL: url('api'), SCOPE_BUCKET_SECRET_ACCESS_KEY: blobs('SECRET_ACCESS_KEY') });
  set('cache', { SCOPE_CACHE_BUCKET_SECRET_ACCESS_KEY: '${{scope-cache-blobs.SECRET_ACCESS_KEY}}' });
  set('media-api', {
    SCOPE_MEDIA_ALLOWED_ORIGIN: url('web'),
    SCOPE_MEDIA_BUCKET_SECRET_ACCESS_KEY: '${{scope-request-media.SECRET_ACCESS_KEY}}',
  });
  set('media-worker', { SCOPE_MEDIA_BUCKET_SECRET_ACCESS_KEY: '${{scope-request-media.SECRET_ACCESS_KEY}}' });
  set('maintenance', {
    SCOPE_BUCKET_NAME: blobs('BUCKET'),
    SCOPE_BUCKET_ENDPOINT: blobs('ENDPOINT'),
    SCOPE_BUCKET_ACCESS_KEY_ID: blobs('ACCESS_KEY_ID'),
    SCOPE_BUCKET_SECRET_ACCESS_KEY: blobs('SECRET_ACCESS_KEY'),
  });

  group([['api', 'SCOPE_OBJECT_ENCRYPTION_KEY'], ['run-worker', 'SCOPE_OBJECT_ENCRYPTION_KEY'],
    ['maintenance', 'SCOPE_OBJECT_ENCRYPTION_KEY']], { SCOPE_OBJECT_ENCRYPTION_KEY: secrets.objectEncryptionKey });
  group([['media-api', 'SCOPE_MEDIA_ENCRYPTION_KEY'], ['media-worker', 'SCOPE_MEDIA_ENCRYPTION_KEY']],
    { SCOPE_MEDIA_ENCRYPTION_KEY: secrets.mediaEncryptionKey });
  group([['api', 'SCOPE_CACHE_GRANT_PRIVATE_KEY'], ['cache', 'SCOPE_CACHE_GRANT_PUBLIC_KEY']], {
    SCOPE_CACHE_GRANT_PRIVATE_KEY: secrets.cacheGrant.privateKey,
    SCOPE_CACHE_GRANT_PUBLIC_KEY: secrets.cacheGrant.publicKey,
  });
  group([['api', 'SCOPE_MEDIA_GRANT_PRIVATE_KEY'], ['media-api', 'SCOPE_MEDIA_GRANT_PUBLIC_KEY']], {
    SCOPE_MEDIA_GRANT_PRIVATE_KEY: secrets.mediaGrant.privateKey,
    SCOPE_MEDIA_GRANT_PUBLIC_KEY: secrets.mediaGrant.publicKey,
  });
  group([['api', 'SCOPE_OPERATOR_TOKEN']], { SCOPE_OPERATOR_TOKEN: secrets.operatorToken });

  return Object.fromEntries(Object.entries(variables).filter(([, values]) => Object.keys(values).length > 0));
}

export function databaseBootstrapPending(maintenanceDatabaseUrl) {
  return maintenanceDatabaseUrl === ADMIN_DATABASE_URL;
}

export function rolePassword(databaseUrl, role) {
  const prefix = `postgresql://${role}:`;
  const password = databaseUrl?.startsWith(prefix) ? databaseUrl.slice(prefix.length).split('@')[0] : '';
  if (!ROLE_PASSWORD.test(password)) throw new Error(`The ${role} database URL does not hold a generated preview password.`);
  return password;
}

export function databaseBootstrap(rolePasswords, migratorPassword) {
  const statements = Object.values(RUNTIME_ROLES).map((role) => [role, rolePasswords[role]]);
  statements.push(['scope_migrator', migratorPassword]);
  const passwords = statements.map(([role, password]) => {
    if (!ROLE_PASSWORD.test(password ?? '')) throw new Error(`A generated password is required for ${role}.`);
    return `ALTER ROLE ${role} PASSWORD '${password}';`;
  });
  return {
    sql: `${renderPolicy({ mode: 'roles' })}${passwords.join('\n')}\n`,
    migratorDatabaseUrl: roleDatabaseUrl('scope_migrator', migratorPassword),
  };
}

const GIT_SEGMENT_SETTINGS = ['SCOPE_GIT_SEGMENT_CHANNEL_CAPACITY', 'SCOPE_GIT_SEGMENT_CHUNK_BYTES',
  'SCOPE_GIT_SEGMENT_MULTIPART_PART_BYTES'];
const BLOB_REFERENCES = ['SCOPE_BUCKET_ACCESS_KEY_ID', 'SCOPE_BUCKET_ENDPOINT', 'SCOPE_BUCKET_NAME', 'SCOPE_BUCKET_REGION'];
const MEDIA_BUCKET = ['SCOPE_MEDIA_BUCKET_ACCESS_KEY_ID', 'SCOPE_MEDIA_BUCKET_ENDPOINT', 'SCOPE_MEDIA_BUCKET_NAME',
  'SCOPE_MEDIA_BUCKET_REGION'];
export const KEPT_STAGING_SETTINGS = Object.freeze({
  api: ['PORT', 'RAILPACK_CONFIG_FILE', 'SCOPE_CACHE_BACKEND', 'SCOPE_DATA_DIR', 'SCOPE_GIT_COMMAND_TIMEOUT_SECS',
    'SCOPE_GIT_RECEIVE_PACK_CONCURRENCY', 'SCOPE_GIT_SEGMENT_INGEST_CONCURRENCY', 'SCOPE_MEDIA_PUBLIC_URL',
    'SCOPE_OBJECT_STORE_MAX_BYTES', 'SCOPE_RUNTIME_TELEMETRY_INTERVAL_SECS', ...GIT_SEGMENT_SETTINGS, ...BLOB_REFERENCES],
  web: ['PAGENT_ENABLED', 'SCOPE_API_INTERNAL_URL', 'SCOPE_CLI_INSTALL_URL', 'VITE_CLERK_SIGN_IN_FALLBACK_REDIRECT_URL',
    'VITE_CLERK_SIGN_IN_URL', 'VITE_CLERK_SIGN_UP_FALLBACK_REDIRECT_URL', 'VITE_CLERK_SIGN_UP_URL',
    'VITE_SCOPE_WEB_FORCE_SIGNED_OUT'],
  cache: ['SCOPE_CACHE_BACKEND', 'SCOPE_CACHE_BUCKET_ACCESS_KEY_ID', 'SCOPE_CACHE_BUCKET_ENDPOINT',
    'SCOPE_CACHE_BUCKET_FORCE_PATH_STYLE', 'SCOPE_CACHE_BUCKET_NAME', 'SCOPE_CACHE_BUCKET_REGION'],
  'run-worker': ['SCOPE_CLOUD_RUNS_ENABLED', 'SCOPE_CLOUD_RUNS_MAX_CONCURRENCY', 'SCOPE_DATA_DIR',
    'SCOPE_GIT_COMPACTION_TIMEOUT_SECS', 'SCOPE_OBJECT_STORE_MAX_BYTES', 'SCOPE_RUNTIME_TELEMETRY_INTERVAL_SECS',
    'SCOPE_RUNTIME_VERSION', ...GIT_SEGMENT_SETTINGS, ...BLOB_REFERENCES],
  'media-api': MEDIA_BUCKET,
  'media-worker': MEDIA_BUCKET,
  'git-router': ['SCOPE_REPO_ROUTER_BACKEND', 'SCOPE_REPO_ROUTER_READ_REPLICAS'],
  'cli-downloads': ['SCOPE_API_PUBLIC_URL', 'SCOPE_CLI_BINARY_PATH', 'SCOPE_CLI_PUBLIC_URL'],
  maintenance: ['SCOPE_BUCKET_REGION'],
  postgres: ['PGDATA', 'PGDATABASE', 'PGHOST', 'PGPORT', 'PGUSER', 'POSTGRES_DB', 'POSTGRES_USER', 'SSL_CERT_DAYS',
    'RAILWAY_DEPLOYMENT_DRAINING_SECONDS'],
});

export function unreviewedStagingVariables(manifest, managed, current) {
  const ids = serviceIds(manifest);
  return Object.fromEntries(Object.entries(ids)
    .map(([component, serviceId]) => {
      const kept = new Set([...(KEPT_STAGING_SETTINGS[component] ?? []), ...Object.keys(managed[serviceId] ?? {})]);
      return [serviceId, Object.keys(current[serviceId] ?? {}).filter((name) => !kept.has(name))];
    })
    .filter(([, names]) => names.length > 0));
}

export function changedVariables(desired, current) {
  return Object.fromEntries(Object.entries(desired)
    .map(([serviceId, values]) => [serviceId, Object.fromEntries(Object.entries(values)
      .filter(([name, value]) => current[serviceId]?.[name] !== value))])
    .filter(([, values]) => Object.keys(values).length > 0));
}
