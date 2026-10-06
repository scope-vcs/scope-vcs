#!/usr/bin/env node

import { execFileSync } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { pathToFileURL } from 'node:url';
import { loadDeploymentManifest } from './railway-artifact.mjs';
import { readRailway } from './railway-read.mjs';
import { RAILWAY_MUTATION_TIMEOUT_MS, retryRailway } from './railway-retry.mjs';
import {
  RUNTIME_ROLES, assertPreviewEnvironment, changedVariables, databaseBootstrap, databaseBootstrapPending,
  generatePreviewSecrets, previewDomains, previewEnvironmentName, previewVariables, releaseEnvironmentIds,
  rolePassword, serviceIds,
} from './preview-environment-plan.mjs';

const POSTGRES_MOUNT = '/var/lib/postgresql/data';
const DEPLOYMENT_TIMEOUT_MS = 900_000;

export function railwayClient() {
  return {
    query(query, variables) {
      return readRailway(['api', query, '--variables', '@-'], { input: JSON.stringify(variables) }).data;
    },
    mutate(query, variables) {
      const result = JSON.parse(execFileSync('railway', ['api', query, '--variables', '@-'], {
        input: JSON.stringify(variables), encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'],
        timeout: RAILWAY_MUTATION_TIMEOUT_MS, killSignal: 'SIGKILL',
      }));
      if (result.errors?.length) throw new Error('Railway GraphQL request failed.');
      return result.data;
    },
    pause(milliseconds) {
      Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, milliseconds);
    },
    now: () => Date.now(),
  };
}

function findEnvironment(railway, projectId, name) {
  const edges = railway.query('query Environments($projectId:String!){project(id:$projectId){environments{edges{node{id name isEphemeral}}}}}',
    { projectId }).project.environments.edges;
  const matches = edges.map(({ node }) => node).filter((environment) => environment.name === name);
  if (matches.length > 1) throw new Error(`Multiple Railway environments are named ${name}.`);
  return matches[0] ?? null;
}

function waitFor(railway, description, check) {
  const deadline = railway.now() + DEPLOYMENT_TIMEOUT_MS;
  for (;;) {
    const result = check();
    if (result) return result;
    if (railway.now() >= deadline) throw new Error(`Timed out waiting for ${description}.`);
    railway.pause(5_000);
  }
}

function environmentConfig(railway, environmentId) {
  return railway.query('query PreviewConfig($environmentId:String!){environment(id:$environmentId){config(decryptVariables:false)}}',
    { environmentId }).environment.config;
}

function currentVariables(railway, projectId, environmentId, ids) {
  return Object.fromEntries(Object.values(ids).map((serviceId) => [serviceId,
    railway.query('query PreviewVariables($projectId:String!,$environmentId:String!,$serviceId:String!){variables(projectId:$projectId,environmentId:$environmentId,serviceId:$serviceId,unrendered:true)}',
      { projectId, environmentId, serviceId }).variables ?? {}]));
}

function upsertVariables(railway, projectId, environmentId, changes) {
  for (const [serviceId, variables] of Object.entries(changes)) {
    retryRailway(() => {
      const data = railway.mutate('mutation PreviewVariables($input:VariableCollectionUpsertInput!){variableCollectionUpsert(input:$input)}',
        { input: { projectId, environmentId, serviceId, variables, skipDeploys: true } });
      if (data?.variableCollectionUpsert !== true) throw new Error('Railway did not confirm preview variables.');
    });
  }
}

function ensurePostgresVolume(railway, projectId, environmentId, serviceId) {
  const volumes = railway.query('query PreviewVolumes($environmentId:String!){environment(id:$environmentId){volumeInstances{edges{node{serviceId mountPath}}}}}',
    { environmentId }).environment.volumeInstances.edges.map(({ node }) => node);
  if (volumes.some((volume) => volume.serviceId === serviceId && volume.mountPath === POSTGRES_MOUNT)) return;
  railway.mutate('mutation PreviewVolume($input:VolumeCreateInput!){volumeCreate(input:$input){id}}',
    { input: { projectId, environmentId, serviceId, mountPath: POSTGRES_MOUNT } });
}

function configureMaintenanceRegistry(railway, environmentId, serviceId, credentials) {
  if (!credentials.username || !credentials.password) throw new Error('Preview maintenance requires registry credentials.');
  retryRailway(() => {
    const data = railway.mutate('mutation PreviewRegistry($serviceId:String!,$environmentId:String!,$input:ServiceInstanceUpdateInput!){serviceInstanceUpdate(serviceId:$serviceId,environmentId:$environmentId,input:$input)}',
      { serviceId, environmentId, input: { registryCredentials: credentials } });
    if (data?.serviceInstanceUpdate !== true) throw new Error('Railway did not confirm preview registry access.');
  });
}

function enableTracing(railway, environmentId, ids) {
  for (const [component, serviceId] of Object.entries(ids)) {
    if (component === 'postgres') continue;
    const input = component === 'web' ? { tracingEnabled: true, autoInstrumentationEnabled: true } : { tracingEnabled: true };
    retryRailway(() => {
      const data = railway.mutate('mutation PreviewTracing($serviceId:String!,$environmentId:String!,$input:ServiceInstanceUpdateInput!){serviceInstanceUpdate(serviceId:$serviceId,environmentId:$environmentId,input:$input)}',
        { serviceId, environmentId, input });
      if (data?.serviceInstanceUpdate !== true) throw new Error(`Railway did not confirm preview tracing for ${component}.`);
    });
  }
}

function latestDeploymentStatus(railway, environmentId, serviceId) {
  return railway.query('query PreviewLatest($environmentId:String!,$serviceId:String!){serviceInstance(environmentId:$environmentId,serviceId:$serviceId){latestDeployment{status}}}',
    { environmentId, serviceId }).serviceInstance?.latestDeployment?.status ?? null;
}

export function deployService(railway, environmentId, serviceId, description) {
  const deploymentId = railway.mutate('mutation PreviewDeploy($serviceId:String!,$environmentId:String!){serviceInstanceDeployV2(serviceId:$serviceId,environmentId:$environmentId)}',
    { serviceId, environmentId }).serviceInstanceDeployV2;
  if (typeof deploymentId !== 'string' || !deploymentId) throw new Error(`Railway did not deploy ${description}.`);
  waitFor(railway, description, () => {
    const status = railway.query('query PreviewDeployment($id:String!){deployment(id:$id){status}}', { id: deploymentId }).deployment?.status;
    if (['FAILED', 'CRASHED', 'REMOVED', 'SKIPPED'].includes(status)) throw new Error(`${description} deployment ${deploymentId} is ${status}.`);
    return status === 'SUCCESS';
  });
}

export function ensurePreviewEnvironment({ manifest, pullRequest, clerk, registryCredentials, railway,
  secrets = generatePreviewSecrets() }) {
  const projectId = manifest.railway.projectId;
  const { staging } = releaseEnvironmentIds(manifest);
  const ids = serviceIds(manifest);
  const name = previewEnvironmentName(pullRequest);
  if (!findEnvironment(railway, projectId, name)) {
    railway.mutate('mutation PreviewCreate($input:EnvironmentCreateInput!){environmentCreate(input:$input){id}}', {
      input: { projectId, name, sourceEnvironmentId: staging, ephemeral: true, skipInitialDeploys: true },
    });
  }
  const environmentId = assertPreviewEnvironment(manifest, waitFor(railway, `${name} creation`,
    () => findEnvironment(railway, projectId, name)), name);
  const config = waitFor(railway, `${name} services`, () => {
    const candidate = environmentConfig(railway, environmentId);
    return Object.values(ids).every((id) => candidate?.services?.[id]) ? candidate : null;
  });
  const domains = previewDomains(manifest, config);
  ensurePostgresVolume(railway, projectId, environmentId, ids.postgres);
  enableTracing(railway, environmentId, ids);
  configureMaintenanceRegistry(railway, environmentId, ids.maintenance, registryCredentials);

  const current = currentVariables(railway, projectId, environmentId, ids);
  const changes = changedVariables(previewVariables({ manifest, domains, current, secrets, clerk }), current);
  upsertVariables(railway, projectId, environmentId, changes);
  for (const component of ['postgres', 'maintenance']) {
    if (changes[ids[component]] || latestDeploymentStatus(railway, environmentId, ids[component]) !== 'SUCCESS') {
      deployService(railway, environmentId, ids[component], `preview ${component}`);
    }
  }
  return { environmentId, name, urls: Object.fromEntries(Object.entries(domains).map(([component, domain]) => [component, `https://${domain}`])) };
}

export function bootstrapPreviewDatabase({ manifest, environmentId, railway, runBootstrap,
  migratorPassword = randomBytes(32).toString('hex') }) {
  const projectId = manifest.railway.projectId;
  const ids = serviceIds(manifest);
  const { production, staging } = releaseEnvironmentIds(manifest);
  if (environmentId === production || environmentId === staging) throw new Error('Database bootstrap only runs in preview environments.');
  const variables = currentVariables(railway, projectId, environmentId, ids);
  if (!databaseBootstrapPending(variables[ids.maintenance].DATABASE_URL)) return { bootstrapped: false };
  const rolePasswords = Object.fromEntries(Object.entries(RUNTIME_ROLES)
    .map(([component, role]) => [role, rolePassword(variables[ids[component]].DATABASE_URL, role)]));
  const { sql, migratorDatabaseUrl } = databaseBootstrap(rolePasswords, migratorPassword);
  runBootstrap(environmentId, sql);
  upsertVariables(railway, projectId, environmentId, { [ids.maintenance]: { DATABASE_URL: migratorDatabaseUrl } });
  deployService(railway, environmentId, ids.maintenance, 'preview maintenance');
  return { bootstrapped: true };
}

export function deletePreviewEnvironment({ manifest, pullRequest, railway }) {
  const name = previewEnvironmentName(pullRequest);
  const environment = findEnvironment(railway, manifest.railway.projectId, name);
  if (!environment) return { deleted: false, name };
  const environmentId = assertPreviewEnvironment(manifest, environment, name);
  const data = railway.mutate('mutation PreviewDelete($id:String!){environmentDelete(id:$id)}', { id: environmentId });
  if (data?.environmentDelete !== true) throw new Error(`Railway did not delete ${name}.`);
  return { deleted: true, name, environmentId };
}

function runBootstrapOverSsh(environmentId, sql) {
  execFileSync('bash', ['-c', 'source "$1"; railway_private_command "$2" sh -ceu \'exec psql "$DATABASE_URL" -X -q -v ON_ERROR_STOP=1\'',
    'preview-bootstrap', new URL('./railway-private-command.sh', import.meta.url).pathname, environmentId], {
    input: sql, stdio: ['pipe', 'ignore', 'inherit'], env: { ...process.env, SCOPE_RAILWAY_PREVIEW_ENVIRONMENT_ID: environmentId },
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const [command, target] = process.argv.slice(2);
    const manifest = loadDeploymentManifest();
    const railway = railwayClient();
    let result;
    if (command === 'ensure') {
      result = ensurePreviewEnvironment({
        manifest, pullRequest: target, railway,
        clerk: { publishableKey: process.env.CLERK_DEVELOPMENT_PUBLISHABLE_KEY, secretKey: process.env.CLERK_DEVELOPMENT_SECRET_KEY },
        registryCredentials: { username: process.env.SCOPE_RAILWAY_REGISTRY_USERNAME, password: process.env.SCOPE_RAILWAY_REGISTRY_PASSWORD },
      });
    } else if (command === 'bootstrap') {
      result = bootstrapPreviewDatabase({ manifest, environmentId: target, railway, runBootstrap: runBootstrapOverSsh });
    } else if (command === 'delete') {
      result = deletePreviewEnvironment({ manifest, pullRequest: target, railway });
    } else {
      throw new Error('usage: preview-environment.mjs <ensure|delete> <pull-request-number> | bootstrap <environment-id>');
    }
    console.log(JSON.stringify(result));
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
