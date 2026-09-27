#!/usr/bin/env node

import { execFileSync } from 'node:child_process';
import { lstatSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { assertDeploymentImage, releaseImageRepository, validatePreparedRelease } from './railway-artifact.mjs';
import { readRailway } from './railway-read.mjs';
import { assertHealthyRailwayService, railwayServicesFromStatus } from './railway-service-health.mjs';

const webBundle = '/app/.output/server/_ssr/ssr.mjs';

export function stagingWebImage(prepared, manifest, repository, status, history, { sourceSha } = {}) {
  validatePreparedRelease(prepared, {
    sourceSha,
    services: manifest.services,
  });
  const environment = manifest.environments?.staging?.environmentId;
  if (!environment || environment === manifest.environments?.production?.environmentId) {
    throw new Error('Staging web manifest requires a distinct staging environment');
  }
  const serviceId = manifest.services?.web?.id;
  const service = assertHealthyRailwayService(railwayServicesFromStatus(status, environment), serviceId);
  if (!Array.isArray(history)) throw new Error('Cannot read staging web deployment history');
  const deployment = history.find(row => row.id === service.deploymentId);
  if (!deployment || deployment.status !== 'SUCCESS') {
    throw new Error('The live staging web deployment has no successful provider record');
  }
  if (deployment.serviceId && deployment.serviceId !== serviceId) {
    throw new Error('Staging web deployment belongs to another service');
  }
  if (deployment.environmentId && deployment.environmentId !== environment) {
    throw new Error('Staging web deployment belongs to another environment');
  }

  const image = prepared.components.web?.image
    ?? deployment.meta?.image ?? deployment.meta?.serviceManifest?.source?.image;
  const trustedRepository = releaseImageRepository(manifest, repository, 'web');
  if (typeof image !== 'string' || !/^sha256:[0-9a-f]{64}$/.test(image.split('@')[1] ?? '')
      || image.split('@')[0] !== trustedRepository) {
    throw new Error('Live staging web image is missing an allowlisted immutable digest');
  }
  const references = [deployment.meta?.image, deployment.meta?.serviceManifest?.source?.image]
    .filter(value => typeof value === 'string' && value);
  if (!references.includes(image)) throw new Error('Live staging web deployment lacks exact image source evidence');
  assertDeploymentImage(image, deployment);
  if (prepared.components.web) {
    validatePreparedRelease(prepared, { components: ['web'], services: manifest.services });
  }
  return image;
}

export function copyWebManifest(image, destination, {
  execute = execFileSync,
  registryUsername = process.env.SCOPE_RAILWAY_REGISTRY_USERNAME,
  registryPassword = process.env.SCOPE_RAILWAY_REGISTRY_PASSWORD,
} = {}) {
  if (registryUsername || registryPassword) {
    if (!registryUsername || !registryPassword) throw new Error('Both registry credentials are required');
  }
  const output = resolve(destination);
  mkdirSync(dirname(output), { recursive: true });
  try {
    const existing = lstatSync(output);
    if (!existing.isFile() || existing.isSymbolicLink()) throw new Error('Web manifest destination must be a regular file path');
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
  }
  const directory = mkdtempSync(join(dirname(output), '.scope-staging-web-'));
  const dockerConfig = join(directory, 'docker-config');
  mkdirSync(dockerConfig);
  const container = `scope-staging-web-${process.pid}-${directory.split('/').at(-1)}`;
  const docker = (args, input) => execute('docker', args, {
    input, env: { ...process.env, DOCKER_CONFIG: dockerConfig },
    stdio: ['pipe', 'pipe', 'pipe'], timeout: 120_000,
  });
  try {
    if (registryUsername) {
      docker(['login', image.split('/')[0], '--username', registryUsername, '--password-stdin'], registryPassword);
    }
    docker(['pull', '--platform', 'linux/amd64', image]);
    docker(['create', '--name', container, '--network', 'none', '--entrypoint', '/bin/false', image]);
    const extracted = join(directory, 'ssr.mjs');
    docker(['cp', `${container}:${webBundle}`, extracted]);
    let source;
    try { source = lstatSync(extracted); } catch (error) {
      if (error.code !== 'ENOENT') throw error;
      throw new Error('Staging web image lacks a compiled server manifest');
    }
    if (!source.isFile() || source.isSymbolicLink() || source.size === 0) {
      throw new Error('Staging web image lacks a regular compiled server manifest');
    }
    renameSync(extracted, output);
  } finally {
    try { docker(['container', 'rm', '--volumes', container]); } catch { /* The container may not exist. */ }
    rmSync(directory, { recursive: true, force: true });
  }
  return output;
}

function main() {
  const [preparedPath, destination] = process.argv.slice(2);
  if (!preparedPath || !destination) {
    throw new Error('usage: extract-staging-web-manifest.mjs <prepared-release.json> <destination>');
  }
  const manifest = JSON.parse(readFileSync(process.env.SCOPE_DEPLOYMENT_MANIFEST || '.github/deployment-services.json'));
  const prepared = JSON.parse(readFileSync(preparedPath));
  const project = manifest.railway?.projectId;
  const environment = manifest.environments?.staging?.environmentId;
  const service = manifest.services?.web?.id;
  if (![project, environment, service].every(value => typeof value === 'string' && value)) {
    throw new Error('Staging web target is incomplete');
  }
  const scope = ['--project', project, '--environment', environment];
  const status = readRailway(['status', ...scope, '--json']);
  const history = readRailway(['deployment', 'list', ...scope, '--service', service, '--limit', '100', '--json']);
  const image = stagingWebImage(prepared, manifest, process.env.GITHUB_REPOSITORY, status, history,
    { sourceSha: process.env.SCOPE_DEPLOYMENT_SOURCE_SHA });
  const output = copyWebManifest(image, destination);
  console.log(`Extracted the live staging web manifest to ${output}`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try { main(); } catch (error) { console.error(error.message); process.exitCode = 1; }
}
