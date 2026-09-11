#!/usr/bin/env node
// Temporary maintenance serving uses the release's API image, never a separate service.
import { execFileSync } from 'node:child_process';
import { readFileSync, writeFileSync, renameSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { RAILWAY_MUTATION_TIMEOUT_MS } from './railway-retry.mjs';

const command = '/app/bin/scope-maintenance serve';
const fields = { build: ['buildCommand', 'rootDirectory', 'railwayConfigFile'], deploy: ['startCommand', 'healthcheckPath', 'healthcheckTimeout', 'preDeployCommand'] };
function validate(gate) {
  const uuid = /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i;
  if (!['projectId', 'environmentId', 'serviceId'].every(key => uuid.test(gate[key] ?? '')) || !/^ghcr\.io\/scope-vcs\/[a-z0-9/_-]+-api@sha256:[a-f0-9]{64}$/.test(gate.image ?? '')) {
    throw new Error('Maintenance gate requires explicit project, environment, service IDs and a pinned API image.');
  }
}
function data(railway, query, variables) {
  const result = railway(query, variables);
  if (result.errors?.length || !result.data) throw new Error('Railway maintenance gate request failed.');
  return result.data;
}
function update(gate, input, railway) {
  const result = data(railway, 'mutation MaintenanceGateConfig($serviceId:String!,$environmentId:String!,$input:ServiceInstanceUpdateInput!){serviceInstanceUpdate(serviceId:$serviceId,environmentId:$environmentId,input:$input)}', { serviceId: gate.serviceId, environmentId: gate.environmentId, input });
  if (result.serviceInstanceUpdate !== true) throw new Error('Railway did not confirm maintenance configuration.');
}
export function snapshotGate(gate, { railway, persist, deployments }) {
  validate(gate);
  if (gate.previous) return gate;
  const result = data(railway, 'query MaintenanceGateSnapshot($environmentId:String!){environment(id:$environmentId){projectId config(decryptVariables:false)}}', { environmentId: gate.environmentId });
  if (result.environment?.projectId !== gate.projectId) throw new Error('Maintenance environment does not belong to the requested project.');
  const config = result.environment.config?.services?.[gate.serviceId];
  if (!config?.source) throw new Error('Maintenance target has no canonical service source.');
  const previous = { source: config.source };
  for (const [section, names] of Object.entries(fields)) {
    previous[section] = Object.fromEntries(names.map(name => [name, (name === 'rootDirectory' ? config.source?.rootDirectory : name === 'railwayConfigFile' ? config.source?.railwayConfigFile : config[section]?.[name]) ?? null]));
  }
  const predecessorIds = deployments ? deployments(gate).filter(value => !['REMOVED', 'FAILED', 'CRASHED'].includes(value.status) && !value.deploymentStopped).map(value => value.id) : [];
  const snapshot = { ...gate, previous, predecessorIds, phase: 'snapshotted', capturedAt: new Date().toISOString() };
  persist(snapshot);
  return snapshot;
}
export function enterGate(gate, { railway, persist, deployments }) {
  validate(gate);
  if (!gate.previous) throw new Error('Persist the maintenance snapshot before activation.');
  if (gate.deploymentId) return gate;
  if (deployments && gate.capturedAt) {
    const matches = deployments(gate).filter(deployment =>
      Date.parse(deployment.createdAt) >= Date.parse(gate.capturedAt) &&
      deployment.serviceId === gate.serviceId &&
      deployment.meta?.serviceManifest?.source?.image === gate.image &&
      deployment.meta?.serviceManifest?.deploy?.startCommand === command &&
      !['REMOVED', 'FAILED', 'CRASHED'].includes(deployment.status));
    if (matches.length > 1) throw new Error('Multiple maintenance deployments match; reconcile before retrying.');
    if (matches.length === 1) {
      gate = { ...gate, deploymentId: matches[0].id, phase: 'deploying' };
      persist(gate);
      return gate;
    }
  }
  if (gate.phase === 'deploying') throw new Error('Maintenance deployment outcome is unknown; reconcile its exact deployment ID before retrying.');
  update(gate, { source: { image: gate.image }, rootDirectory: '/', railwayConfigFile: null, buildCommand: null, startCommand: command, healthcheckPath: '/readyz', healthcheckTimeout: 60, preDeployCommand: [] }, railway);
  // Persist uncertainty before the non-idempotent deployment mutation.
  gate = { ...gate, phase: 'deploying' };
  persist(gate);
  const result = data(railway, 'mutation MaintenanceGateDeploy($serviceId:String!,$environmentId:String!){serviceInstanceDeployV2(serviceId:$serviceId,environmentId:$environmentId)}', { serviceId: gate.serviceId, environmentId: gate.environmentId });
  if (typeof result.serviceInstanceDeployV2 !== 'string' || !result.serviceInstanceDeployV2) throw new Error('Railway did not return the maintenance deployment ID.');
  gate = { ...gate, deploymentId: result.serviceInstanceDeployV2 };
  persist(gate);
  return gate;
}
export function recloseGate(gate, options) {
  validate(gate);
  if (!gate.previous) throw new Error('Original maintenance snapshot is required for recovery.');
  const inventory = options.deployments(gate);
  if (gate.phase === 'deploying' && !gate.deploymentId) {
    // Reconcile the persisted attempt before considering a replacement. An empty
    // inventory is not evidence that an accepted deployment never happened.
    gate = enterGate(gate, { ...options, deployments: () => inventory });
  }
  const stopped = value => ['REMOVED', 'FAILED', 'CRASHED'].includes(value.status) || value.deploymentStopped === true;
  const current = inventory.filter(value => !stopped(value));
  const matches = current.filter(value => value.serviceId === gate.serviceId && value.meta?.serviceManifest?.source?.image === gate.image && value.meta?.serviceManifest?.deploy?.startCommand === command);
  if (matches.length > 1) throw new Error('Multiple active maintenance deployments require reconciliation.');
  if (!matches.length && gate.deploymentId) {
    const previous = inventory.find(value => value.id === gate.deploymentId)
      ?? data(options.railway, 'query MaintenancePreviousGate($id:String!){deployment(id:$id){id serviceId status deploymentStopped}}', { id: gate.deploymentId }).deployment;
    if (previous?.id !== gate.deploymentId || previous.serviceId !== gate.serviceId || !stopped(previous)) {
      throw new Error('Previous maintenance deployment outcome is unknown; reconcile its exact deployment ID before retrying.');
    }
  }
  const predecessorIds = current.filter(value => value.id !== matches[0]?.id).map(value => value.id);
  const reset = { ...gate, predecessorIds, capturedAt: new Date().toISOString(), phase: matches.length ? 'deploying' : 'snapshotted' };
  delete reset.deploymentId;
  if (matches.length) reset.deploymentId = matches[0].id;
  options.persist(reset);
  return enterGate(reset, options);
}
export function stopGatePredecessors(gate, { railway }) {
  for (const id of gate.predecessorIds ?? []) {
    if (id === gate.deploymentId) throw new Error('Refusing to stop the maintenance deployment.');
    const deployment = data(railway, 'query MaintenancePredecessor($id:String!){deployment(id:$id){id status serviceId deploymentStopped}}', { id }).deployment;
    if (deployment?.serviceId !== gate.serviceId) throw new Error('Predecessor belongs to another service.');
    if (deployment.status === 'REMOVED' || deployment.deploymentStopped === true) continue;
    const result = data(railway, 'mutation MaintenanceStopPredecessor($id:String!){deploymentStop(id:$id)}', { id });
    if (result.deploymentStop !== true) throw new Error('Railway did not confirm predecessor stop.');
  }
}
export function restoreGateConfiguration(gate, { railway, persist }) {
  validate(gate);
  if (!gate.previous) throw new Error('Maintenance snapshot is missing.');
  const source = Object.fromEntries(['image', 'repo'].filter(key => Object.hasOwn(gate.previous.source, key)).map(key => [key, gate.previous.source[key]]));
  update(gate, { source, ...gate.previous.build, ...gate.previous.deploy, preDeployCommand: gate.previous.deploy.preDeployCommand ?? [] }, railway);
  const restored = { ...gate, phase: 'restored' };
  persist(restored);
  return restored;
}
export function verifyGateDeployment(gate, deployment) {
  if (deployment?.id !== gate.deploymentId || deployment.serviceId !== gate.serviceId || deployment.status !== 'SUCCESS') throw new Error('The exact maintenance deployment is not ready.');
  const manifest = deployment.meta?.serviceManifest;
  if (manifest?.source?.image !== gate.image && deployment.meta?.imageDigest !== gate.image.split('@')[1]) throw new Error('Maintenance deployment image does not match the pinned API image.');
  if (manifest?.deploy?.startCommand !== command || manifest?.deploy?.healthcheckPath !== '/readyz') throw new Error('Maintenance deployment configuration was not activated.');
}
function railway(query, variables) {
  return JSON.parse(execFileSync('railway', ['api', query, '--variables', '@-', '--compact'], { input: JSON.stringify(variables), encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'], timeout: RAILWAY_MUTATION_TIMEOUT_MS, killSignal: 'SIGKILL' }));
}
async function main() {
  const [action, file, extra] = process.argv.slice(2);
  if (!file || extra || !['snapshot', 'enter', 'reclose', 'restore'].includes(action)) throw new Error('usage: railway-maintenance-gate.mjs snapshot|enter|reclose|restore <snapshot.json>');
  let gate = JSON.parse(readFileSync(file, 'utf8'));
  const persist = value => { writeFileSync(`${file}.tmp`, `${JSON.stringify(value)}\n`, { mode: 0o600 }); renameSync(`${file}.tmp`, file); };
  const deployments = value => JSON.parse(execFileSync('railway', ['deployment', 'list', '--project', value.projectId, '--environment', value.environmentId, '--service', value.serviceId, '--limit', '20', '--json'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], timeout: RAILWAY_MUTATION_TIMEOUT_MS, killSignal: 'SIGKILL' })).map(deployment => ({ ...deployment, serviceId: deployment.serviceId ?? value.serviceId }));
  const options = { railway, persist, deployments };
  if (action === 'snapshot') snapshotGate(gate, options);
  if (action === 'restore') restoreGateConfiguration(gate, options);
  if (!['enter', 'reclose'].includes(action)) return;
  gate = action === 'reclose' ? recloseGate(gate, options) : enterGate(gate, options);
  const deadline = Date.now() + 900_000;
  while (Date.now() < deadline) {
    const result = data(railway, 'query MaintenanceDeployment($id:String!){deployment(id:$id){id serviceId status meta}}', { id: gate.deploymentId });
    const deployment = result.deployment;
    if (['FAILED', 'CRASHED', 'REMOVED'].includes(deployment?.status)) throw new Error(`Maintenance deployment is ${deployment.status}.`);
    if (deployment?.status === 'SUCCESS') {
      verifyGateDeployment(gate, deployment);
      stopGatePredecessors(gate, options);
      const remaining = deployments(gate).filter(value => (gate.predecessorIds ?? []).includes(value.id) && value.status !== 'REMOVED' && value.deploymentStopped !== true);
      if (remaining.length) {
        await new Promise(resolve => setTimeout(resolve, 10_000));
        continue;
      }
      persist({ ...gate, phase: 'active' });
      return;
    }
    await new Promise(resolve => setTimeout(resolve, 10_000));
  }
  throw new Error('Maintenance deployment readiness timed out.');
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) main().catch(error => { console.error(error.message); process.exitCode = 1; });
