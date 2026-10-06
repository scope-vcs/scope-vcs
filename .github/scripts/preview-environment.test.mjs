import assert from 'node:assert/strict';
import { createPrivateKey, createPublicKey } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import {
  KEPT_STAGING_SETTINGS, RUNTIME_ROLES, changedVariables, databaseBootstrap, generatePreviewSecrets, previewEnvironmentName,
  previewVariables, rolePassword, serviceIds, unreviewedStagingVariables,
} from './preview-environment-plan.mjs';
import { bootstrapPreviewDatabase, deletePreviewEnvironment, ensurePreviewEnvironment } from './preview-environment.mjs';

const manifest = JSON.parse(readFileSync(new URL('../deployment-services.json', import.meta.url), 'utf8'));
const ids = serviceIds(manifest);
const production = manifest.environments.production.environmentId;
const staging = manifest.environments.staging.environmentId;
const preview = '99999999-9999-4999-8999-999999999999';
const clerk = { publishableKey: `pk_test_${Buffer.from('happy-otter-12.clerk.accounts.dev$').toString('base64url')}`, secretKey: 'sk_test_preview' };
const registryCredentials = { username: 'registry-user', password: 'registry-password' };

function stagingCopy() {
  const services = Object.fromEntries(Object.values(ids).map((id) => [id, {}]));
  for (const component of ['api', 'web', 'cache', 'git-router', 'media-api']) {
    services[ids[component]] = { networking: { serviceDomains: { [`${manifest.services[component].name}-pr-7.up.railway.app`]: {} } } };
  }
  const variables = Object.fromEntries(Object.values(ids).map((id) => [id, {}]));
  Object.assign(variables[ids.api], {
    SCOPE_API_PUBLIC_URL: 'https://scope-api-staging.up.railway.app',
    SCOPE_RESEND_API_KEY: 're_staging_secret',
    SCOPE_BUCKET_ACCESS_KEY_ID: '${{scope-blobs.ACCESS_KEY_ID}}',
    SCOPE_GIT_COMMAND_TIMEOUT_SECS: '60',
  });
  Object.assign(variables[ids.web], { PAGENT_SOURCE_TOKEN: 'pagent-staging-token', PAGENT_ENABLED: 'false' });
  Object.assign(variables[ids['run-worker']], { SCOPE_ANALYTICS_DSN: 'postgresql://reader:staging-pw@${{scope-postgres.RAILWAY_PRIVATE_DOMAIN}}/x' });
  Object.assign(variables[ids.cache], { SCOPE_CACHE_GRANT_PUBLIC_KEY: 'staging-public-key' });
  Object.assign(variables[ids['media-api']], { SCOPE_MEDIA_GRANT_PUBLIC_KEY: 'staging-public-key' });
  Object.assign(variables[ids.maintenance], { SCOPE_BUCKET_NAME: 'scope-blobs-staging' });
  return { services, variables };
}

function fakeRailway({ environments = [] } = {}) {
  const state = { environments: [...environments], volumes: [], tracing: {}, calls: [], deployments: [], bootstraps: [], copy: stagingCopy(), clock: 0 };
  const railway = {
    state,
    now: () => state.clock,
    pause: (milliseconds) => { state.clock += milliseconds; },
    query(query, variables) {
      if (query.startsWith('query Environments')) {
        assert.equal(variables.projectId, manifest.railway.projectId);
        return { project: { environments: { edges: state.environments.map((node) => ({ node })) } } };
      }
      if (query.startsWith('query PreviewConfig')) return { environment: { config: { services: state.copy.services } } };
      if (query.startsWith('query PreviewVariables')) {
        assert.equal(variables.environmentId, preview);
        return { variables: { ...state.copy.variables[variables.serviceId] } };
      }
      if (query.startsWith('query PreviewVolumes')) {
        return { environment: { volumeInstances: { edges: state.volumes.map((node) => ({ node })) } } };
      }
      if (query.startsWith('query PreviewLatest')) {
        const deployed = state.deployments.some((serviceId) => serviceId === variables.serviceId);
        return { serviceInstance: { latestDeployment: deployed ? { status: 'SUCCESS' } : null } };
      }
      if (query.startsWith('query PreviewDeployment')) return { deployment: { status: 'SUCCESS' } };
      throw new Error(`unexpected query ${query}`);
    },
    mutate(query, variables) {
      state.calls.push([query.split('(')[0], variables]);
      if (query.startsWith('mutation PreviewCreate')) {
        assert.deepEqual(variables.input, { projectId: manifest.railway.projectId, name: 'pr-7', sourceEnvironmentId: staging, ephemeral: true, skipInitialDeploys: true });
        state.environments.push({ id: preview, name: 'pr-7', isEphemeral: true });
        return { environmentCreate: { id: preview } };
      }
      if (query.startsWith('mutation PreviewVolume')) {
        state.volumes.push({ serviceId: variables.input.serviceId, mountPath: variables.input.mountPath });
        return { volumeCreate: { id: 'volume' } };
      }
      if (query.startsWith('mutation PreviewRegistry')) return { serviceInstanceUpdate: true };
      if (query.startsWith('mutation PreviewTracing')) {
        assert.equal(variables.environmentId, preview);
        state.tracing[variables.serviceId] = variables.input;
        return { serviceInstanceUpdate: true };
      }
      if (query.startsWith('mutation PreviewVariables')) {
        const { serviceId, variables: values, environmentId, skipDeploys } = variables.input;
        assert.equal(environmentId, preview);
        assert.equal(skipDeploys, true);
        Object.assign(state.copy.variables[serviceId], values);
        return { variableCollectionUpsert: true };
      }
      if (query.startsWith('mutation PreviewVariableDelete')) {
        const { serviceId, name, environmentId } = variables.input;
        assert.equal(environmentId, preview);
        delete state.copy.variables[serviceId][name];
        return { variableDelete: true };
      }
      if (query.startsWith('mutation PreviewDeploy')) {
        state.deployments.push(variables.serviceId);
        return { serviceInstanceDeployV2: `deployment-${state.deployments.length}` };
      }
      if (query.startsWith('mutation PreviewDelete')) {
        state.environments = state.environments.filter((environment) => environment.id !== variables.id);
        return { environmentDelete: true };
      }
      throw new Error(`unexpected mutation ${query}`);
    },
  };
  return railway;
}

function ensure(railway) {
  return ensurePreviewEnvironment({ manifest, pullRequest: '7', clerk, registryCredentials, railway });
}

function bootstrap(railway, environmentId = preview) {
  return bootstrapPreviewDatabase({
    manifest, environmentId, railway,
    runBootstrap: (target, sql) => railway.state.bootstraps.push({ environmentId: target, sql }),
  });
}

test('names preview environments only from positive pull request numbers', () => {
  assert.equal(previewEnvironmentName('42'), 'pr-42');
  for (const value of ['0', '-1', '01', '4.2', 'main', '1234567890', '']) {
    assert.throws(() => previewEnvironmentName(value), /positive pull request/);
  }
});

test('generated secrets have the formats the services parse', () => {
  const secrets = generatePreviewSecrets();
  assert.equal(Buffer.from(secrets.objectEncryptionKey, 'base64').length, 32);
  assert.equal(Buffer.from(secrets.mediaEncryptionKey, 'base64').length, 32);
  for (const pair of [secrets.cacheGrant, secrets.mediaGrant]) {
    assert.equal(createPrivateKey(pair.privateKey).asymmetricKeyType, 'ed25519');
    assert.equal(createPublicKey(pair.privateKey).export({ type: 'spki', format: 'pem' }), pair.publicKey);
  }
  assert.deepEqual(Object.keys(secrets.rolePasswords).sort(), Object.keys(RUNTIME_ROLES).sort());
  assert.notEqual(generatePreviewSecrets().objectEncryptionKey, secrets.objectEncryptionKey);
});

test('first provisioning points every service at preview domains, buckets, and fresh keys', () => {
  const secrets = generatePreviewSecrets();
  const domains = { api: 'a.up.railway.app', web: 'w.up.railway.app', cache: 'c.up.railway.app', 'git-router': 'g.up.railway.app', 'media-api': 'm.up.railway.app' };
  const variables = previewVariables({ manifest, domains, current: stagingCopy().variables, secrets, clerk });
  assert.equal(variables[ids.api].SCOPE_APP_ORIGIN, 'https://w.up.railway.app');
  assert.equal(variables[ids.api].CLERK_AUTHORIZED_PARTIES, 'https://w.up.railway.app');
  assert.equal(variables[ids.api].SCOPE_GIT_PUBLIC_URL, 'https://g.up.railway.app');
  assert.equal(variables[ids.web].CLERK_SECRET_KEY, 'sk_test_preview');
  assert.equal(variables[ids.web].VITE_CLERK_PUBLISHABLE_KEY, clerk.publishableKey);
  assert.equal(variables[ids.api].CLERK_ISSUER, 'https://happy-otter-12.clerk.accounts.dev');
  assert.equal(variables[ids.api].CLERK_JWKS_URL, 'https://happy-otter-12.clerk.accounts.dev/.well-known/jwks.json');
  assert.equal(variables[ids.web].SCOPE_API_HOST, 'a.up.railway.app');
  assert.equal(variables[ids['run-worker']].SCOPE_PUBLIC_API_URL, 'https://a.up.railway.app');
  assert.equal(variables[ids['media-api']].SCOPE_MEDIA_ALLOWED_ORIGIN, 'https://w.up.railway.app');
  assert.equal(variables[ids.maintenance].SCOPE_BUCKET_NAME, '${{scope-blobs.BUCKET}}');
  assert.equal(variables[ids.maintenance].DATABASE_URL, '${{scope-postgres.DATABASE_URL}}');
  assert.equal(variables[ids.cache].SCOPE_CACHE_GRANT_PUBLIC_KEY, secrets.cacheGrant.publicKey);
  assert.equal(variables[ids['media-api']].SCOPE_MEDIA_GRANT_PUBLIC_KEY, secrets.mediaGrant.publicKey);
  for (const component of ['api', 'run-worker', 'maintenance']) {
    assert.equal(variables[ids[component]].SCOPE_OBJECT_ENCRYPTION_KEY, secrets.objectEncryptionKey);
  }
  for (const [component, role] of Object.entries(RUNTIME_ROLES)) {
    assert.equal(rolePassword(variables[ids[component]].DATABASE_URL, role), secrets.rolePasswords[component]);
  }
  assert.doesNotMatch(JSON.stringify(variables), /staging/);
  for (const invalid of [{}, { ...clerk, secretKey: 'sk_live_production' },
    { publishableKey: `pk_live_${Buffer.from('clerk.scopevcs.com$').toString('base64url')}`, secretKey: 'sk_test_preview' },
    { publishableKey: `pk_test_${Buffer.from('evil.example.com$').toString('base64url')}`, secretKey: 'sk_test_preview' }]) {
    assert.throws(() => previewVariables({ manifest, domains, current: {}, secrets, clerk: invalid }), /Clerk development/);
  }
});

test('later provisioning keeps keys and passwords that already protect preview data', () => {
  const domains = { api: 'a', web: 'w', cache: 'c', 'git-router': 'g', 'media-api': 'm' };
  const first = previewVariables({ manifest, domains, current: stagingCopy().variables, secrets: generatePreviewSecrets(), clerk });
  const current = Object.fromEntries(Object.values(ids).map((id) => [id, { ...stagingCopy().variables[id], ...first[id] }]));
  const second = previewVariables({ manifest, domains, current, secrets: generatePreviewSecrets(), clerk });
  assert.deepEqual(changedVariables(second, current), {});
});

test('database bootstrap creates roles before migrations and sets every generated password', () => {
  const password = (character) => character.repeat(64);
  const passwords = Object.fromEntries(Object.values(RUNTIME_ROLES).map((role, index) => [role, password(String(index))]));
  const { sql, migratorDatabaseUrl } = databaseBootstrap(passwords, password('f'));
  assert.match(sql, /CREATE ROLE scope_migrator LOGIN/);
  assert.match(sql, /ALTER DATABASE %I OWNER TO scope_migrator/);
  assert.doesNotMatch(sql, /GRANT SELECT/);
  for (const [role, value] of Object.entries(passwords)) assert.match(sql, new RegExp(`ALTER ROLE ${role} PASSWORD '${value}';`));
  assert.equal(rolePassword(migratorDatabaseUrl, 'scope_migrator'), password('f'));
  assert.throws(() => databaseBootstrap({ ...passwords, scope_api: "x'; DROP ROLE postgres; --" }, password('f')), /generated password/);
});

test('creates, configures, and bootstraps a preview copy of staging once', () => {
  const railway = fakeRailway();
  const result = ensure(railway);
  assert.deepEqual(bootstrap(railway), { bootstrapped: true });
  assert.equal(result.environmentId, preview);
  assert.equal(result.urls.web, 'https://scope-web-pr-7.up.railway.app');
  assert.deepEqual(railway.state.volumes, [{ serviceId: ids.postgres, mountPath: '/var/lib/postgresql/data' }]);
  assert.equal(railway.state.tracing[ids.postgres], undefined);
  assert.deepEqual(railway.state.tracing[ids.web], { tracingEnabled: true, autoInstrumentationEnabled: true });
  for (const component of ['api', 'run-worker', 'cache', 'git-router', 'media-api', 'media-worker', 'maintenance']) {
    assert.deepEqual(railway.state.tracing[ids[component]], { tracingEnabled: true });
  }
  assert.deepEqual(railway.state.deployments, [ids.postgres, ids.maintenance, ids.maintenance]);
  assert.equal(railway.state.bootstraps.length, 1);
  assert.equal(railway.state.bootstraps[0].environmentId, preview);
  assert.doesNotMatch(JSON.stringify(railway.state.copy.variables), /re_staging_secret|pagent-staging-token|staging-pw/);
  assert.equal(railway.state.copy.variables[ids.api].SCOPE_BUCKET_ACCESS_KEY_ID, '${{scope-blobs.ACCESS_KEY_ID}}');
  assert.equal(railway.state.copy.variables[ids.api].SCOPE_GIT_COMMAND_TIMEOUT_SECS, '60');
  assert.equal(railway.state.copy.variables[ids.web].PAGENT_ENABLED, 'false');
  assert.equal(rolePassword(railway.state.copy.variables[ids.maintenance].DATABASE_URL, 'scope_migrator').length, 64);

  const before = structuredClone(railway.state.copy.variables);
  const calls = railway.state.calls.length;
  ensure(railway);
  assert.deepEqual(bootstrap(railway), { bootstrapped: false });
  assert.deepEqual(railway.state.copy.variables, before);
  assert.equal(railway.state.bootstraps.length, 1);
  assert.deepEqual([...new Set(railway.state.calls.slice(calls).map(([name]) => name))], ['mutation PreviewTracing', 'mutation PreviewRegistry']);
});

test('refuses release and persistent environments that share the preview name', () => {
  for (const environment of [{ id: staging, name: 'pr-7', isEphemeral: true }, { id: production, name: 'pr-7', isEphemeral: true },
    { id: preview, name: 'pr-7', isEphemeral: false }]) {
    const railway = fakeRailway({ environments: [environment] });
    assert.throws(() => ensure(railway), /not an ephemeral preview environment/);
    assert.throws(() => deletePreviewEnvironment({ manifest, pullRequest: '7', railway }), /not an ephemeral preview environment/);
    assert.deepEqual(railway.state.calls, []);
  }
  for (const environmentId of [production, staging]) {
    const railway = fakeRailway();
    assert.throws(() => bootstrap(railway, environmentId), /only runs in preview/);
    assert.deepEqual(railway.state.bootstraps, []);
  }
});

test('deletes only the pull request preview and tolerates an absent one', () => {
  const railway = fakeRailway({ environments: [{ id: preview, name: 'pr-7', isEphemeral: true }, { id: staging, name: 'staging', isEphemeral: false }] });
  assert.deepEqual(deletePreviewEnvironment({ manifest, pullRequest: '7', railway }), { deleted: true, name: 'pr-7', environmentId: preview });
  assert.deepEqual(railway.state.environments.map(({ name }) => name), ['staging']);
  assert.deepEqual(deletePreviewEnvironment({ manifest, pullRequest: '7', railway }), { deleted: false, name: 'pr-7' });
});

test('previews keep only reviewed staging settings and their own variables', () => {
  const managed = previewVariables({ manifest, domains: { api: 'a', web: 'w', cache: 'c', 'git-router': 'g', 'media-api': 'm' },
    current: {}, secrets: generatePreviewSecrets(), clerk });
  const current = stagingCopy().variables;
  current[ids.api].SCOPE_FUTURE_SETTING = 'unreviewed';
  const removed = unreviewedStagingVariables(manifest, managed, current);
  assert.deepEqual(removed[ids.api].sort(), ['SCOPE_FUTURE_SETTING', 'SCOPE_RESEND_API_KEY']);
  assert.deepEqual(removed[ids.web], ['PAGENT_SOURCE_TOKEN']);
  assert.deepEqual(removed[ids['run-worker']], ['SCOPE_ANALYTICS_DSN']);
  assert.equal(removed[ids.maintenance], undefined);
  for (const names of Object.values(KEPT_STAGING_SETTINGS)) {
    assert.ok(names.every((name) => !/SECRET|TOKEN|PASSWORD|PRIVATE/.test(name)), names.join(','));
  }
});

test('stops provisioning when Railway cannot list a service variables', () => {
  const railway = fakeRailway();
  const query = railway.query;
  railway.query = (text, variables) => text.startsWith('query PreviewVariables') ? { variables: null } : query(text, variables);
  assert.throws(() => ensure(railway), /did not return preview variables/);
  assert.ok(!railway.state.calls.some(([name]) => name.startsWith('mutation PreviewVariable')));
});
