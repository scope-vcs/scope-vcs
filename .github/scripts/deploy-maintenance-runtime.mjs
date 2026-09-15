#!/usr/bin/env node
import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { assertDeploymentImage } from './railway-artifact.mjs';
import { readRailway } from './railway-read.mjs';
import { RAILWAY_MUTATION_TIMEOUT_MS } from './railway-retry.mjs';

const runtimeImage = /^ghcr\.io\/scope-vcs\/scope-vcs\/railway-private-maintenance@sha256:[a-f0-9]{64}$/;
const apiImage = /^ghcr\.io\/scope-vcs\/scope-vcs\/railway-private-api@sha256:[a-f0-9]{64}$/;
const uuid = /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i;
const startCommand = '/app/bin/scope-maintenance serve';

export function validateReceipt(receipt) {
  const fields = ['schemaVersion', 'image', 'maintenanceSha256', 'apiImage', 'apiSourceSha', 'runtimeSourceSha'];
  if (!receipt || Object.keys(receipt).length !== fields.length || !fields.every(key => Object.hasOwn(receipt, key)) ||
      !fields.slice(1).every(key => typeof receipt[key] === 'string') ||
      receipt.schemaVersion !== 1 || !runtimeImage.test(receipt.image) || !apiImage.test(receipt.apiImage) ||
      !/^[a-f0-9]{64}$/.test(receipt.maintenanceSha256) ||
      !['apiSourceSha', 'runtimeSourceSha'].every(key => /^[a-f0-9]{40}$/.test(receipt[key]))) {
    throw new Error('Maintenance receipt requires schema version 1, pinned runtime/API images, binary SHA256, and source commits.');
  }
  return receipt;
}

export function maintenanceTarget(manifest, environment, env = {}) {
  if (!['staging', 'production'].includes(environment)) throw new Error('Maintenance environment must be staging or production.');
  const target = { projectId: manifest.railway?.projectId, serviceId: manifest.railway?.maintenanceServiceId, environmentId: manifest.environments?.[environment]?.environmentId };
  const environmentIds = ['staging', 'production'].map(name => manifest.environments?.[name]?.environmentId);
  if (!Object.values(target).every(value => typeof value === 'string' && uuid.test(value)) ||
      !environmentIds.every(value => typeof value === 'string' && uuid.test(value)) || environmentIds[0] === environmentIds[1] ||
      target.serviceId === manifest.railway?.databaseServiceId ||
      Object.values(manifest.services ?? {}).some(service => service.id === target.serviceId)) {
    throw new Error('Maintenance target must be a dedicated service with distinct, explicit environment IDs.');
  }
  for (const [key, value] of Object.entries({ RAILWAY_PROJECT_ID: target.projectId, SCOPE_RAILWAY_ENVIRONMENT_ID: target.environmentId, RAILWAY_ENVIRONMENT_ID: target.environmentId, SCOPE_RAILWAY_MAINTENANCE_SERVICE_ID: target.serviceId, RAILWAY_SERVICE_ID: target.serviceId })) {
    if (env[key] && env[key] !== value) throw new Error(`Maintenance target conflicts with ${key}.`);
  }
  return target;
}

function request(railway, query, variables) {
  const response = railway(query, variables);
  if (response?.errors?.length || !response?.data) throw new Error('Railway maintenance runtime request failed.');
  return response.data;
}

export function verifyRuntimeDeployment(target, receipt, deploymentId, deployment) {
  if (deployment?.id !== deploymentId || deployment.serviceId !== target.serviceId || deployment.environmentId !== target.environmentId || deployment.status !== 'SUCCESS' || deployment.deploymentStopped === true) {
    throw new Error('The exact maintenance runtime deployment is not ready in the requested service and environment.');
  }
  const manifest = deployment.meta?.serviceManifest;
  assertDeploymentImage(receipt.image, deployment);
  if (manifest?.deploy?.startCommand !== startCommand || manifest?.deploy?.healthcheckPath !== '/readyz') {
    throw new Error('Maintenance runtime deployment image, start command, or health check does not match.');
  }
}

export async function deployMaintenanceRuntime({ manifest, environment, receipt, credentials, env = {} }, {
  railway, report = message => console.error(message), now = Date.now, pause = milliseconds => new Promise(resolve => setTimeout(resolve, milliseconds)), timeoutMs = 900_000,
}) {
  validateReceipt(receipt);
  const target = maintenanceTarget(manifest, environment, env);
  if (!['username', 'password'].every(key => typeof credentials?.[key] === 'string' && credentials[key].trim())) throw new Error('Durable Railway registry credentials are required.');
  const actual = request(railway, 'query MaintenanceRuntimeTarget($environmentId:String!){environment(id:$environmentId){projectId config(decryptVariables:false)}}', { environmentId: target.environmentId }).environment;
  if (actual?.projectId !== target.projectId || !Object.hasOwn(actual.config?.services ?? {}, target.serviceId)) throw new Error('Maintenance service is absent from the requested project and environment.');

  const updated = request(railway, 'mutation MaintenanceRuntimeConfig($serviceId:String!,$environmentId:String!,$input:ServiceInstanceUpdateInput!){serviceInstanceUpdate(serviceId:$serviceId,environmentId:$environmentId,input:$input)}', {
    serviceId: target.serviceId, environmentId: target.environmentId,
    input: { source: { image: receipt.image }, rootDirectory: '/', railwayConfigFile: null, buildCommand: null, startCommand, healthcheckPath: '/readyz', healthcheckTimeout: 60, preDeployCommand: [], registryCredentials: credentials },
  });
  if (updated.serviceInstanceUpdate !== true) throw new Error('Railway did not confirm maintenance runtime configuration.');
  // Setting repo:null alongside image clears Railway's source selection. Select
  // only the image, then verify the stored source before starting any deployment.
  const configured = request(railway, 'query MaintenanceRuntimeSource($environmentId:String!){environment(id:$environmentId){projectId config(decryptVariables:false)}}', { environmentId: target.environmentId }).environment;
  const source = configured?.config?.services?.[target.serviceId]?.source;
  if (configured?.projectId !== target.projectId || source?.image !== receipt.image || source?.repo) throw new Error('Railway did not retain the pinned maintenance image source.');
  // Never retry this non-idempotent mutation after an uncertain response.
  const deployed = request(railway, 'mutation MaintenanceRuntimeDeploy($serviceId:String!,$environmentId:String!){serviceInstanceDeployV2(serviceId:$serviceId,environmentId:$environmentId)}', { serviceId: target.serviceId, environmentId: target.environmentId });
  const deploymentId = deployed.serviceInstanceDeployV2;
  if (typeof deploymentId !== 'string' || !deploymentId) throw new Error('Railway did not return an exact maintenance runtime deployment ID.');
  report(`Maintenance runtime deployment ID: ${deploymentId}`);

  const deadline = now() + timeoutMs;
  while (now() < deadline) {
    const deployment = request(railway, 'query MaintenanceRuntimeDeployment($id:String!){deployment(id:$id){id serviceId environmentId status deploymentStopped meta}}', { id: deploymentId }).deployment;
    if (deployment && (deployment.id !== deploymentId || deployment.serviceId !== target.serviceId || deployment.environmentId !== target.environmentId)) throw new Error('Railway returned a maintenance deployment from another target.');
    if (['FAILED', 'CRASHED', 'REMOVED', 'SKIPPED'].includes(deployment?.status) || deployment?.deploymentStopped === true) throw new Error('Maintenance runtime deployment failed or stopped.');
    if (deployment?.status === 'SUCCESS') {
      verifyRuntimeDeployment(target, receipt, deploymentId, deployment);
      return { ...target, ...receipt, deploymentId };
    }
    await pause(10_000);
  }
  throw new Error('Maintenance runtime deployment readiness timed out.');
}

export function railwayClient(env, execute = execFileSync) {
  const isolated = { ...env };
  if (isolated.RAILWAY_API_TOKEN) delete isolated.RAILWAY_TOKEN;
  if (!isolated.RAILWAY_API_TOKEN && !isolated.RAILWAY_TOKEN) throw new Error('A Railway mutation token is required.');
  return (query, variables) => {
    const input = JSON.stringify(variables);
    const run = (command, args, options) => execute(command, args, { ...options, env: isolated });
    if (query.startsWith('query ')) return readRailway(['api', query, '--variables', '@-'], { input, execute: run });
    try {
      return JSON.parse(run('railway', ['api', query, '--variables', '@-', '--compact'], { input, encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'], timeout: RAILWAY_MUTATION_TIMEOUT_MS, killSignal: 'SIGKILL' }));
    } catch {
      throw new Error('Railway maintenance runtime mutation failed; its outcome may be unknown.');
    }
  };
}

async function main() {
  const [environment, receiptFile, extra] = process.argv.slice(2);
  if (!receiptFile || extra) throw new Error('usage: deploy-maintenance-runtime.mjs staging|production RECEIPT_JSON');
  const receipt = JSON.parse(readFileSync(receiptFile, 'utf8'));
  const manifest = JSON.parse(readFileSync(process.env.SCOPE_DEPLOYMENT_MANIFEST || '.github/deployment-services.json', 'utf8'));
  const result = await deployMaintenanceRuntime({
    manifest, environment, receipt, env: process.env,
    credentials: { username: process.env.SCOPE_RAILWAY_REGISTRY_USERNAME, password: process.env.SCOPE_RAILWAY_REGISTRY_PASSWORD },
  }, {
    railway: railwayClient(process.env),
  });
  process.stdout.write(`${JSON.stringify(result)}\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main().catch(error => { console.error(error.message); process.exitCode = 1; });
