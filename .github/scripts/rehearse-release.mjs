#!/usr/bin/env node

import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdir, readFile, writeFile, access, rename } from 'node:fs/promises';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { verifyStagingTarget } from './verify-staging-target.mjs';
import { readRailway } from './railway-read.mjs';

const deploymentOrder = ['router', 'cache', 'worker', 'api', 'web'];

export function previousDeploymentsRemoved(previous, deployments) {
  return previous.every(({ serviceId, deploymentId }) => {
    const deployment = deployments[serviceId]?.find(({ id }) => id === deploymentId);
    // Missing history is not evidence that the old container was torn down.
    return deployment?.status === 'REMOVED';
  });
}

function processTask(command, args, env = process.env) {
  const child = spawn(command, args, { env, stdio: 'inherit' });
  const done = new Promise((accept, reject) => {
    child.once('error', reject);
    child.once('exit', (code, signal) => code === 0
      ? accept()
      : reject(new Error(`${command} failed: ${signal ?? code}`)));
  });
  // Background browser errors remain observable when the deployment finishes.
  done.catch(() => {});
  return { child, done };
}

async function waitForFile(path, task) {
  const deadline = Date.now() + 90_000;
  while (Date.now() < deadline) {
    try { await access(path); return; } catch {}
    if (task.child.exitCode !== null || task.child.signalCode) {
      await task.done;
      throw new Error(`Browser exited before writing ${path}`);
    }
    await delay(250);
  }
  throw new Error(`Timed out waiting for ${path}`);
}

async function main() {
  assert(process.env.RAILWAY_TOKEN && !process.env.RAILWAY_API_TOKEN,
    'Release rehearsal requires only a staging-scoped Railway token');
  const manifest = JSON.parse(await readFile(process.env.SCOPE_DEPLOYMENT_MANIFEST ?? '.github/deployment-services.json', 'utf8'));
  const { railway, services } = manifest;
  const sourceSha = process.env.SCOPE_DEPLOYMENT_SOURCE_SHA;
  assert.match(sourceSha ?? '', /^[a-f0-9]{40}$/);
  assert(process.env.SCOPE_PREPARED_RELEASE_PATH, 'Prepared release manifest is required');
  const prepared = JSON.parse(await readFile(process.env.SCOPE_PREPARED_RELEASE_PATH, 'utf8'));
  assert.equal(prepared.sourceSha, sourceSha, 'Prepared images must match the requested revision');
  const components = deploymentOrder.filter((component) => prepared.components?.[component]);
  assert(components.length > 0, 'Prepared release has no staging application components');
  const provesActivity = process.env.SCOPE_REHEARSAL_IMPORTED !== '1';
  const scope = ['--project', railway.projectId, '--environment', railway.staging.environmentId];
  const query = async (...args) => readRailway([...args, ...scope, '--json']);
  verifyStagingTarget({ manifest, status: await query('status'), services: await query('service', 'list') });
  const env = {
    ...process.env,
    RAILWAY_PROJECT_ID: railway.projectId,
    SCOPE_RAILWAY_ENVIRONMENT_ID: railway.staging.environmentId,
    SCOPE_WEB_BASE_URL: `https://${railway.staging.webDomain}`,
    GITHUB_SHA: sourceSha,
  };
  const output = resolve(process.env.SCOPE_REHEARSAL_EVIDENCE_DIR ?? 'release-rehearsal');
  await mkdir(output, { recursive: true });

  if (process.argv[2] !== 'transition') {
    for (let rotation = 1; rotation <= 3; rotation += 1) {
      const directory = resolve(output, `ordinary-${rotation}`);
      await mkdir(directory);
      const configPath = resolve(directory, 'monitor.json');
      await writeFile(configPath, JSON.stringify({
        mode: 'ordinary', webOrigin: env.SCOPE_WEB_BASE_URL,
        apiOrigin: `https://${railway.staging.apiDomain}`,
        intervalMs: 1000, requestTimeoutMs: 5000,
        release: { attemptId: `staging-rehearsal-${rotation}`, stage: `staging-ordinary-${rotation}`, sourceSha, deploymentsFile: resolve(directory, 'active-deployments.json') },
        fixture: { owner: 'dev', repo: 'update-demo', filePath: 'README.md', expectedText: '# Update Demo', expectedRequestIds: ['req_demo_ready'] },
      }, null, 2));
      await processTask('bash', ['.github/scripts/with-release-availability.sh', configPath, directory,
        '--', process.execPath, '.github/scripts/rehearse-release.mjs', 'transition', String(rotation), directory], env).done;
    }
    await writeFile(resolve(output, 'summary.json'), JSON.stringify({ sourceSha, ordinaryTransitions: 3, passed: true }));
    return;
  }

  const rotation = Number(process.argv[3]);
  assert([1, 2, 3].includes(rotation));
  const directory = resolve(process.argv[4]);
  const readyPath = resolve(directory, 'browser.ready.json');
  const teardownPath = resolve(directory, 'old-teardown');
  const activationPath = resolve(directory, 'activation-started');
  const activityPath = resolve(directory, 'expected-activity');
  const updateReadyPath = resolve(directory, 'browser-update.ready.json');
  const browser = processTask(process.execPath, ['web/smoke/release-transition.mjs',
    '--base-url', env.SCOPE_WEB_BASE_URL, '--require-sse-reconnect', String(components.includes('api')), '--repo', 'dev/update-demo', '--ready-file', readyPath, '--activation-file', activationPath, '--transition-file', teardownPath,
    '--summary', resolve(directory, 'browser-summary.json'),
    ...(rotation === 3 && provesActivity ? [
      '--expected-activity-file', activityPath,
      '--update-ready-file', updateReadyPath,
    ] : [])], env);
  try {
    await waitForFile(readyPath, browser);
    const current = await query('service', 'list');
    const previous = components.map((component) => {
      const serviceId = component === 'router' ? railway.staging.routerServiceId : services[component].id;
      const live = current.find(({ id }) => id === serviceId);
      assert(live?.deploymentId && live.status === 'SUCCESS', `${component} must have a healthy predecessor`);
      return { component, serviceId, deploymentId: live.deploymentId };
    });
    await writeFile(resolve(directory, 'predecessors.json'), JSON.stringify(previous, null, 2));
    const active = Object.fromEntries(previous.map(({ component, deploymentId }) => [component, deploymentId]));
    const activePath = resolve(directory, 'active-deployments.json');
    const recordActive = async () => {
      await writeFile(`${activePath}.tmp`, JSON.stringify(active));
      await rename(`${activePath}.tmp`, activePath);
    };
    await recordActive();
    await writeFile(activationPath, new Date().toISOString());
    for (const { component, serviceId } of previous) {
      await processTask('bash', ['.github/scripts/deploy-railway.sh', serviceId,
        component === 'cache' ? 'cache-service' : component === 'router' ? 'repo-router' : component], {
        ...env, SCOPE_DEPLOYMENT_COMPONENT: component,
        SCOPE_DEPLOYMENT_EVIDENCE_PATH: resolve(directory, 'deployments.ndjson'),
      }).done;
      const records = (await readFile(resolve(directory, 'deployments.ndjson'), 'utf8'))
        .trim().split('\n').map(JSON.parse);
      const evidence = records.findLast((record) => record.component === component);
      assert(evidence?.evidenceId && evidence.evidenceId !== active[component],
        `${component} must create a new deployment of the prepared image`);
      active[component] = evidence.evidenceId;
      await recordActive();
    }
    const deadline = Date.now() + 300_000;
    while (true) {
      const deployments = Object.fromEntries(await Promise.all(previous.map(async ({ serviceId }) =>
        [serviceId, await query('deployment', 'list', '--service', serviceId, '--limit', '10')])));
      await writeFile(resolve(directory, 'teardown-status.json'), JSON.stringify(deployments, null, 2));
      if (previousDeploymentsRemoved(previous, deployments)) break;
      assert(Date.now() < deadline, 'Old deployments did not reach REMOVED before the teardown deadline');
      await delay(5000);
    }
    await writeFile(teardownPath, new Date().toISOString());
    if (rotation === 3 && provesActivity) {
      await waitForFile(updateReadyPath, browser);
      await processTask('bash', ['.github/scripts/staging-git-smoke.sh'], {
        ...env,
        SCOPE_API_URL: `https://${railway.staging.apiDomain}`,
        SCOPE_GIT_ROUTER_URL: `https://${railway.staging.routerDomain}`,
        SCOPE_CLI_BINARY: resolve('cli/target/release/scope'),
        SCOPE_EXCHANGE_TOKEN_PATH: resolve(process.env.SCOPE_GIT_SMOKE_DIR, 'exchange-token'),
      }).done;
      await writeFile(activityPath, 'Exercise the staging Git router');
    }
    await browser.done;
  } finally {
    if (browser.child.exitCode === null && !browser.child.signalCode) browser.child.kill('SIGTERM');
    await browser.done.catch(() => {});
  }
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => { console.error(error.message); process.exitCode = 1; });
}
