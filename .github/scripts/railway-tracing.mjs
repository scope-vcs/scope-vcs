import { pathToFileURL } from 'node:url';
import { loadDeploymentManifest } from './railway-artifact.mjs';
import { serviceIds } from './deployment-components.mjs';
import { railwayClient } from './railway-client.mjs';
import { retryRailway } from './railway-retry.mjs';

export function disableRailwayTracing(railway, environmentId, ids) {
  for (const [component, serviceId] of Object.entries(ids)) {
    if (component === 'postgres') continue;
    const input = component === 'web'
      ? { tracingEnabled: false, autoInstrumentationEnabled: false }
      : { tracingEnabled: false };
    retryRailway(() => {
      const data = railway.mutate('mutation DisableRailwayTracing($serviceId:String!,$environmentId:String!,$input:ServiceInstanceUpdateInput!){serviceInstanceUpdate(serviceId:$serviceId,environmentId:$environmentId,input:$input)}',
        { serviceId, environmentId, input });
      if (data?.serviceInstanceUpdate !== true) throw new Error('Railway did not confirm disabling tracing.');
    });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const manifest = loadDeploymentManifest();
    if (process.argv.length !== 3 || process.argv[2] !== 'production' ||
        process.env.SCOPE_RAILWAY_ENVIRONMENT_ID !== manifest.environments.production.environmentId ||
        process.env.RAILWAY_PROJECT_ID !== manifest.railway.projectId) {
      throw new Error('Disabling Railway tracing requires the manifest production target.');
    }
    disableRailwayTracing(railwayClient(), manifest.environments.production.environmentId, serviceIds(manifest));
    console.log('Production Railway tracing disabled; services export to PostHog after deployment.');
  } catch (error) {
    console.error(`Disabling production Railway tracing failed: ${error.message} Release activation blocked.`);
    process.exitCode = 1;
  }
}
