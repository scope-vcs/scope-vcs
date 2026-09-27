import { execFileSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';
import { RAILWAY_COMPONENTS } from './deployment-components.mjs';
import { railwayServicesFromStatus, verifyProductionRailwayServices } from './railway-service-health.mjs';
import { renderRuntimeRoleAudit } from '../../deploy/postgres/audit-runtime-roles.mjs';

const rolePolicyPath = 'deploy/postgres/runtime-roles.mjs';

export function readDeployedFile(sha, path) {
  if (!/^[a-f0-9]{40}$/.test(sha ?? '')) throw new Error('Production baseline requires an exact source SHA');
  return execFileSync('git', ['show', `${sha}:${path}`], { encoding: 'utf8' });
}

export function deployedReadinessBaseline(deployments, read = readDeployedFile) {
  const manifests = new Map();
  const services = {};
  const serviceConfigs = {};
  for (const component of RAILWAY_COMPONENTS) {
    const sha = deployments?.[component]?.sourceSha;
    if (!/^[a-f0-9]{40}$/.test(sha ?? '')) throw new Error(`Production ${component} has no source revision`);
    if (!manifests.has(sha)) manifests.set(sha, JSON.parse(read(sha, '.github/deployment-services.json')));
    const definition = manifests.get(sha).services?.[component];
    if (!definition?.id || !definition.deployment?.runtimeConfig) {
      throw new Error(`Production ${component} is missing its deployed service definition`);
    }
    services[component] = definition;
    serviceConfigs[component] = JSON.parse(read(sha, definition.deployment.runtimeConfig));
  }
  return { services, serviceConfigs, rolePolicy: read(deployments.api.sourceSha, rolePolicyPath) };
}

export function productionReadinessAudit({ deployments, manifest, status, baseline,
  candidateRolePolicy = readFileSync(rolePolicyPath, 'utf8') }) {
  verifyProductionRailwayServices({
    deployments,
    manifest: { ...manifest, services: baseline.services },
    serviceConfigs: baseline.serviceConfigs,
    services: railwayServicesFromStatus(status, manifest.environments.production.environmentId),
  });
  // A role-policy change is a release input, not existing production drift.
  // Retain role/ownership/ledger checks before that new grant policy is applied.
  return renderRuntimeRoleAudit({ exactPolicy: candidateRolePolicy === baseline.rolePolicy });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const deployments = JSON.parse(process.env.SCOPE_PRODUCTION_DEPLOYMENTS_JSON);
    process.stdout.write(productionReadinessAudit({
      deployments,
      manifest: JSON.parse(process.env.SCOPE_DEPLOYMENT_MANIFEST_JSON),
      status: JSON.parse(process.env.SCOPE_RAILWAY_SERVICES_JSON),
      baseline: deployedReadinessBaseline(deployments),
    }));
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
