#!/usr/bin/env node

import { readFileSync, readdirSync, renameSync, writeFileSync } from 'node:fs';
import { execFile } from 'node:child_process';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { promisify } from 'node:util';
import { readRailway } from './railway-read.mjs';

const idPattern = /^[A-Za-z0-9-]+$/;
const execFileAsync = promisify(execFile);

async function boundedRead(args, remainingMs) {
  try {
    const { stdout } = await execFileAsync(process.execPath,
      [fileURLToPath(new URL('./railway-read.mjs', import.meta.url)), ...args],
      { timeout: Math.max(1, remainingMs), killSignal: 'SIGKILL' });
    return JSON.parse(stdout);
  } catch {
    throw new Error('Railway predecessor inventory read failed');
  }
}

function scope() {
  const project = process.env.RAILWAY_PROJECT_ID;
  const environment = process.env.SCOPE_RAILWAY_ENVIRONMENT_ID;
  if (!project || !environment) throw new Error('Railway project and environment are required for predecessor teardown');
  return { project, environment };
}

export function activePredecessors(status, environment, service) {
  const environments = status?.environments?.edges?.map(({ node }) => node)
    .filter((node) => node.id === environment || node.name === environment);
  if (environments?.length !== 1) throw new Error('Railway environment is missing or ambiguous');
  const instances = environments[0].serviceInstances?.edges?.map(({ node }) => node)
    .filter((node) => node.serviceId === service || node.serviceName === service);
  if (instances?.length !== 1) throw new Error('Railway service is missing or ambiguous');
  const active = instances[0].activeDeployments;
  if (!Array.isArray(active) || active.some(({ id }) => typeof id !== 'string' || !idPattern.test(id))) {
    throw new Error('Railway active deployments are invalid');
  }
  return [...new Set(active.map(({ id }) => id))];
}

export function recordPredecessors(directory, component, service, ids) {
  if (!/^[a-z][a-z-]*$/.test(component) || !idPattern.test(service)
      || !Array.isArray(ids) || ids.some((id) => typeof id !== 'string' || !idPattern.test(id))) {
    throw new Error('Invalid predecessor snapshot');
  }
  // One component owns one file. The snapshot is written before any provider mutation.
  writeFileSync(join(directory, `${component}.json`), `${JSON.stringify({ component, service, ids: [...new Set(ids)] })}\n`, { flag: 'wx' });
}

export function excludeActivated(directory, component, deploymentId) {
  if (!/^[a-z][a-z-]*$/.test(component) || !idPattern.test(deploymentId)) {
    throw new Error('Invalid activated deployment identity');
  }
  const path = join(directory, `${component}.json`);
  const record = JSON.parse(readFileSync(path, 'utf8'));
  if (record.component !== component || !Array.isArray(record.ids)) throw new Error('Invalid predecessor snapshot');
  if (!record.ids.includes(deploymentId)) return;
  const temporary = `${path}.${process.pid}.tmp`;
  writeFileSync(temporary, `${JSON.stringify({ ...record, ids: record.ids.filter((id) => id !== deploymentId) })}\n`, { flag: 'wx' });
  renameSync(temporary, path);
}

export async function waitForPredecessors(directory, {
  read = boundedRead,
  now = Date.now,
  pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms)),
  timeoutMs = 600_000,
  intervalMs = 5_000,
} = {}) {
  const { project, environment } = scope();
  const files = readdirSync(directory).filter((name) => name.endsWith('.json'));
  const records = files.map((name) => JSON.parse(readFileSync(join(directory, name), 'utf8')));
  const pending = new Map();
  for (const { component, service, ids } of records) {
    if (!/^[a-z][a-z-]*$/.test(component) || !idPattern.test(service)
        || !Array.isArray(ids) || ids.some((id) => !idPattern.test(id)) || pending.has(service)) {
      throw new Error('Invalid predecessor snapshot');
    }
    if (ids.length) pending.set(service, new Set(ids));
  }
  const deadline = now() + timeoutMs;
  while (pending.size) {
    for (const [service, ids] of pending) {
      if (now() > deadline) break;
      const deployments = await read([
        'deployment', 'list', '--project', project, '--environment', environment,
        '--service', service, '--limit', '100', '--json',
      ], deadline - now());
      if (!Array.isArray(deployments)) throw new Error(`Invalid deployment inventory for ${service}`);
      for (const id of ids) {
        if (deployments.find((deployment) => deployment.id === id)?.status === 'REMOVED') {
          ids.delete(id);
          console.log(`Previous deployment ${id} completed teardown.`);
        }
      }
      if (!ids.size) pending.delete(service);
    }
    if (!pending.size) return;
    if (now() >= deadline) {
      throw new Error(`Previous deployments have not completed teardown: ${[...pending.values()].flatMap((ids) => [...ids]).join(', ')}`);
    }
    await pause(intervalMs);
  }
}

async function main() {
  const [command, directory, component, service, idsJson] = process.argv.slice(2);
  if (command === 'record') {
    recordPredecessors(directory, component, service, JSON.parse(idsJson));
  } else if (command === 'snapshot') {
    const { project, environment } = scope();
    const status = readRailway(['status', '--project', project, '--environment', environment, '--json']);
    recordPredecessors(directory, component, service, activePredecessors(status, environment, service));
  } else if (command === 'activated') {
    excludeActivated(directory, component, service);
  } else if (command === 'wait') {
    const timeout = Number(process.env.SCOPE_PREDECESSOR_TEARDOWN_TIMEOUT_SECONDS ?? 600);
    const interval = Number(process.env.SCOPE_PREDECESSOR_TEARDOWN_POLL_SECONDS ?? 5);
    if (!Number.isInteger(timeout) || timeout < 0 || !Number.isInteger(interval) || interval < 0) {
      throw new Error('Invalid predecessor teardown timeout');
    }
    await waitForPredecessors(directory, { timeoutMs: timeout * 1000, intervalMs: interval * 1000 });
  } else {
    throw new Error('usage: railway-predecessor-teardown.mjs record|snapshot|wait ...');
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => { console.error(error.message); process.exitCode = 1; });
}
