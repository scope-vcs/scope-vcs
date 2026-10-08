import { pathToFileURL } from 'node:url';
import { loadDeploymentManifest } from './railway-artifact.mjs';
import { serviceIds } from './deployment-components.mjs';
import { railwayClient } from './railway-client.mjs';
import { retryRailway } from './railway-retry.mjs';

export function configureTracing(railway, environmentId, ids) {
  for (const [component, serviceId] of Object.entries(ids)) {
    if (component === 'postgres') continue;
    const input = component === 'web'
      ? { tracingEnabled: true, autoInstrumentationEnabled: true }
      : { tracingEnabled: true };
    retryRailway(() => {
      const data = railway.mutate('mutation ConfigureTracing($serviceId:String!,$environmentId:String!,$input:ServiceInstanceUpdateInput!){serviceInstanceUpdate(serviceId:$serviceId,environmentId:$environmentId,input:$input)}',
        { serviceId, environmentId, input });
      if (data?.serviceInstanceUpdate !== true) throw new Error('Railway did not confirm tracing configuration.');
    });
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const manifest = loadDeploymentManifest();
    if (process.argv.length !== 3 || process.argv[2] !== 'production' ||
        process.env.SCOPE_RAILWAY_ENVIRONMENT_ID !== manifest.environments.production.environmentId ||
        process.env.RAILWAY_PROJECT_ID !== manifest.railway.projectId) {
      throw new Error('Tracing configuration requires the manifest production target.');
    }
    configureTracing(railwayClient(), manifest.environments.production.environmentId, serviceIds(manifest));
    console.log('Production application tracing configured; exporter activation requires deployment.');
  } catch {
    console.error('Production tracing configuration failed; release activation blocked.');
    process.exitCode = 1;
  }
}
