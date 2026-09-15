import assert from 'node:assert/strict';
import { readFileSync, writeFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { assertActivatedArtifact, releaseImageRepository, verifyPrivateImagePackage } from './railway-artifact.mjs';
import { readRailway } from './railway-read.mjs';

export function currentApiManifest(manifest, repository, { image, sourceSha, maintenanceSha256 }, deployments) {
  assert.match(sourceSha ?? '', /^[a-f0-9]{40}$/);
  assert.match(maintenanceSha256 ?? '', /^[a-f0-9]{64}$/);
  assert.equal(image?.split('@')[0], releaseImageRepository(manifest, repository, 'api'));
  assert.match(image ?? '', /@sha256:[a-f0-9]{64}$/);
  const active = deployments.filter(row => !row.deploymentStopped && !['REMOVED', 'FAILED', 'CRASHED'].includes(row.status));
  assert.equal(active.length, 1, 'Maintenance preparation requires one stable active production API deployment');
  const release = { schemaVersion: 1, sourceSha, maintenanceSha256, components: {
    api: { image, sourceSha, serviceId: manifest.services.api.id },
  } };
  assertActivatedArtifact(release, 'api', active[0], { deploymentId: active[0].id });
  if (active[0].environmentId) assert.equal(active[0].environmentId, manifest.environments.production.environmentId);
  return release;
}

async function main() {
  const [action, path] = process.argv.slice(2);
  const manifest = JSON.parse(readFileSync(process.env.SCOPE_DEPLOYMENT_MANIFEST || '.github/deployment-services.json'));
  const repository = process.env.GITHUB_REPOSITORY;
  if (action === 'source') {
    const deployments = readRailway(['deployment', 'list', '--project', manifest.railway.projectId,
      '--environment', manifest.environments.production.environmentId, '--service', manifest.services.api.id,
      '--limit', '20', '--json']);
    const release = currentApiManifest(manifest, repository, {
      image: process.env.CURRENT_API_IMAGE, sourceSha: process.env.CURRENT_API_SOURCE_SHA,
      maintenanceSha256: process.env.MAINTENANCE_SHA256,
    }, deployments);
    writeFileSync(path, `${JSON.stringify(release)}\n`);
  } else if (['verify-package', 'verify-publish-target'].includes(action)) {
    await verifyPrivateImagePackage(`ghcr.io/${repository.toLowerCase()}/${manifest.railway.releaseImagePrefix}-maintenance`, repository,
      { token: process.env.GITHUB_TOKEN, allowMissing: action === 'verify-publish-target' });
  } else throw new Error('Usage: maintenance-runtime.mjs source <manifest> | verify-package');
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
