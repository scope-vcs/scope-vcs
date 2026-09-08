import { readFileSync, writeFileSync, renameSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { execFileSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';
import { readRailway } from './railway-read.mjs';

const componentPaths = { api: 'api', worker: 'worker', cache: 'cache-service', router: 'repo-router', web: 'web', cli: 'cli' };
const componentBinaries = { api: 'scope-vcs', worker: 'scope-worker', cache: 'scope-cache-service', router: 'scope-repo-router', cli: 'scope-cli-service' };
const digestReference = /^[a-z0-9][a-z0-9./_-]*@sha256:[a-f0-9]{64}$/;
const sourceRevision = /^[a-f0-9]{40}$/;

export function releaseImageRepository(manifest, repository, component) {
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]*\/[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(repository ?? '')) {
    throw new Error('Release images require a trusted OWNER/REPOSITORY.');
  }
  const prefix = manifest?.railway?.releaseImagePrefix;
  if (typeof prefix !== 'string' || !/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(prefix)) {
    throw new Error('Deployment manifest requires a valid railway.releaseImagePrefix.');
  }
  if (!Object.hasOwn(componentPaths, component)) throw new Error(`Unknown release component ${component}.`);
  return `ghcr.io/${repository.toLowerCase()}/${prefix}-${component}`;
}

export async function verifyPrivateReleasePackage(manifest, repository, component, { token, fetchImpl = fetch } = {}) {
  const imageRepository = releaseImageRepository(manifest, repository, component);
  if (!token) throw new Error('GITHUB_TOKEN is required to verify private package visibility.');
  const owner = repository.split('/')[0].toLowerCase();
  const packageName = imageRepository.slice(`ghcr.io/${owner}/`.length);
  async function metadata(path) {
    const response = await fetchImpl(`https://api.github.com${path}`, {
      headers: { Authorization: `Bearer ${token}`, Accept: 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28' },
      redirect: 'error', signal: AbortSignal.timeout(15000),
    });
    if (!response.ok) throw new Error(`GitHub package metadata request failed with HTTP ${response.status}.`);
    return response.json();
  }
  const account = await metadata(`/users/${owner}`);
  if (account.login?.toLowerCase() !== owner || !['Organization', 'User'].includes(account.type)) {
    throw new Error('GitHub did not confirm the release package owner.');
  }
  const namespace = account.type === 'Organization' ? 'orgs' : 'users';
  const packageInfo = await metadata(`/${namespace}/${owner}/packages/container/${encodeURIComponent(packageName)}`);
  if (packageInfo.name !== packageName || packageInfo.package_type !== 'container' || packageInfo.owner?.login?.toLowerCase() !== owner) {
    throw new Error('GitHub returned a different release package.');
  }
  if (packageInfo.visibility !== 'private') throw new Error(`Release package ${imageRepository} must be private.`);
  return { imageRepository, visibility: 'private' };
}

export function validatePreparedRelease(release, { sourceSha, components = [], services } = {}) {
  if (!release || release.schemaVersion !== 1 || !sourceRevision.test(release.sourceSha ?? '')) {
    throw new Error('Prepared release requires schemaVersion 1 and a full sourceSha.');
  }
  if (release.maintenanceSha256 !== undefined && !/^[a-f0-9]{64}$/.test(release.maintenanceSha256)) throw new Error('Invalid maintenance binary digest.');
  if (release.preparationRunId !== undefined && !/^[0-9]+$/.test(release.preparationRunId)) throw new Error('Invalid preparation run ID.');
  if (sourceSha && sourceSha !== release.sourceSha) throw new Error('Prepared release source revision does not match the candidate.');
  if (!release.components || Array.isArray(release.components) || typeof release.components !== 'object') {
    throw new Error('Prepared release components are missing.');
  }
  for (const component of components) {
    if (!release.components[component]) throw new Error(`Prepared release is missing ${component}.`);
  }
  for (const [component, artifact] of Object.entries(release.components)) {
    if (!Object.hasOwn(componentPaths, component)) throw new Error(`Unknown release component ${component}.`);
    if (!artifact || artifact.sourceSha !== release.sourceSha || !digestReference.test(artifact.image ?? '') ||
        typeof artifact.serviceId !== 'string' || !artifact.serviceId.trim()) {
      throw new Error(`Prepared ${component} must bind its source revision, service ID, and immutable image digest.`);
    }
    if (services && services[component]?.id !== artifact.serviceId) throw new Error(`Prepared ${component} targets the wrong service.`);
  }
  return release;
}

export function validateMaintenanceArtifact(release, binaryBuffer) {
  validatePreparedRelease(release);
  if (!release.maintenanceSha256) throw new Error('Prepared release is missing its maintenance binary digest.');
  if (!Buffer.isBuffer(binaryBuffer) || binaryBuffer.length === 0) throw new Error('Maintenance binary is missing or empty.');
  const actual = createHash('sha256').update(binaryBuffer).digest('hex');
  if (actual !== release.maintenanceSha256) throw new Error('Maintenance binary does not match the prepared release digest.');
  return actual;
}

export function assertActivatedArtifact(release, component, deployment, { deploymentId } = {}) {
  validatePreparedRelease(release, { components: [component] });
  if (!deploymentId || deployment?.id !== deploymentId) throw new Error('Artifact verification requires the exact activated deployment ID.');
  if (deployment.status !== 'SUCCESS') throw new Error('Artifact deployment has not reached SUCCESS.');
  const artifact = release.components[component];
  if (deployment.serviceId && deployment.serviceId !== artifact.serviceId) throw new Error('Artifact deployment belongs to another service.');
  const expectedDigest = artifact.image.split('@')[1];
  const meta = deployment.meta ?? {};
  const references = [meta.image, meta.serviceManifest?.source?.image].filter((value) => typeof value === 'string' && value);
  if (references.some((value) => value !== artifact.image)) throw new Error('Activated deployment image differs from the prepared artifact.');
  if (meta.imageDigest && meta.imageDigest !== expectedDigest) throw new Error('Activated deployment digest differs from the prepared artifact.');
  if (meta.imageDigest !== expectedDigest && !references.includes(artifact.image)) {
    throw new Error('Activated deployment has no immutable image evidence.');
  }
  return deployment;
}

export function artifactDeploymentInput(component, artifact, config, { registryCredentials } = {}) {
  if (!componentPaths[component] || !digestReference.test(artifact.image ?? '')) throw new Error('Invalid immutable Railway artifact.');
  const deploy = config?.deploy;
  if (!deploy?.healthcheckPath || !Number.isInteger(deploy.healthcheckTimeout)) {
    throw new Error(`Checked-in readiness configuration is missing for ${component}.`);
  }
  const input = {
    source: { image: artifact.image },
    rootDirectory: '/', railwayConfigFile: null, buildCommand: null,
    startCommand: component === 'web' ? 'node /app/.output/server/index.mjs' : `/app/bin/${componentBinaries[component]}`,
    healthcheckPath: deploy.healthcheckPath,
    healthcheckTimeout: deploy.healthcheckTimeout,
    preDeployCommand: [],
  };
  // Replica topology belongs to the target environment, not the artifact.
  for (const field of ['overlapSeconds', 'drainingSeconds', 'restartPolicyMaxRetries']) {
    if (deploy[field] === undefined) continue;
    const value = deploy[field];
    if ((typeof value !== 'number' && !(typeof value === 'string' && /^[0-9]+$/.test(value))) ||
        !Number.isInteger(Number(value)) || Number(value) < 0 || Number(value) > 2147483647) {
      throw new Error(`Checked-in deploy.${field} must be a nonnegative GraphQL Int.`);
    }
    input[field] = Number(value);
  }
  if (deploy.restartPolicyType !== undefined) input.restartPolicyType = deploy.restartPolicyType;
  if (registryCredentials) {
    if (!registryCredentials.username || !registryCredentials.password) throw new Error('Both registry username and password are required.');
    input.registryCredentials = registryCredentials;
  }
  return input;
}

export function configureStagingRegistry(manifest, credentials, railway = runRailway) {
  if (!credentials?.username && !credentials?.password) return { configured: false };
  if (!credentials.username || !credentials.password) throw new Error('Both registry username and password are required.');
  const environmentId = manifest?.railway?.staging?.environmentId;
  const productionId = manifest?.railway?.environmentId;
  const uuid = /^[a-f0-9]{8}(?:-[a-f0-9]{4}){3}-[a-f0-9]{12}$/i;
  if (!uuid.test(environmentId ?? '') || !uuid.test(productionId ?? '') || environmentId === productionId) {
    throw new Error('Registry configuration requires a distinct, explicit staging environment.');
  }
  const serviceIds = ['cache', 'worker', 'router', 'api', 'web'].map((component) => {
    const id = component === 'router' ? manifest.railway.staging.routerServiceId : manifest.services?.[component]?.id;
    if (!uuid.test(id ?? '')) throw new Error(`Staging registry configuration is missing ${component} service ID.`);
    return id;
  });
  if (new Set(serviceIds).size !== serviceIds.length) throw new Error('Staging registry service IDs must be distinct.');
  // Configure only provider-held pull credentials. Candidate activation receives
  // the staging token and retains these credentials without ever reading them.
  for (const serviceId of serviceIds) {
    const result = railway('mutation ConfigureRegistry($serviceId:String!,$environmentId:String!,$input:ServiceInstanceUpdateInput!){serviceInstanceUpdate(serviceId:$serviceId,environmentId:$environmentId,input:$input)}', {
      serviceId, environmentId, input: { registryCredentials: credentials },
    });
    if (result.data?.serviceInstanceUpdate !== true) throw new Error('Railway did not confirm staging registry configuration.');
  }
  return { configured: true, serviceCount: serviceIds.length };
}

export function activateArtifact(release, component, environmentId, { config, services, registryCredentials, railway = runRailway } = {}) {
  validatePreparedRelease(release, { components: [component], services });
  if (!environmentId) throw new Error('An explicit Railway environment ID is required.');
  const artifact = release.components[component];
  const variables = { serviceId: artifact.serviceId, environmentId, input: artifactDeploymentInput(component, artifact, config, { registryCredentials }) };
  const updated = railway('mutation ActivateSource($serviceId:String!,$environmentId:String!,$input:ServiceInstanceUpdateInput!){serviceInstanceUpdate(serviceId:$serviceId,environmentId:$environmentId,input:$input)}', variables);
  if (updated.data?.serviceInstanceUpdate !== true) throw new Error('Railway did not confirm the environment source update.');
  const selected = railway('query SelectedSource($serviceId:String!,$environmentId:String!){serviceInstance(serviceId:$serviceId,environmentId:$environmentId){source{image repo}}}', variables);
  if (selected.data?.serviceInstance?.source?.image !== artifact.image || selected.data?.serviceInstance?.source?.repo) {
    throw new Error('Railway source read-back did not match the prepared image.');
  }
  const canonical = railway('query SelectedConfig($environmentId:String!){environment(id:$environmentId){config(decryptVariables:false)}}', { environmentId });
  const canonicalSource = canonical.data?.environment?.config?.services?.[artifact.serviceId]?.source;
  if (canonicalSource?.image !== artifact.image || canonicalSource?.repo) throw new Error('Railway canonical environment config did not select the prepared image.');
  const deployed = railway('mutation ActivateImage($serviceId:String!,$environmentId:String!){serviceInstanceDeployV2(serviceId:$serviceId,environmentId:$environmentId)}', variables);
  const deploymentId = deployed.data?.serviceInstanceDeployV2;
  if (typeof deploymentId !== 'string' || !deploymentId) throw new Error('Railway did not return an exact deployment ID.');
  return { deploymentId, component, sourceSha: release.sourceSha, image: artifact.image, serviceId: artifact.serviceId };
}

function runRailway(query, variables) {
  // Variables go through stdin to keep private registry credentials out of process arguments.
  const args = ['api', query, '--variables', '@-'];
  if (query.startsWith('query ')) return readRailway(args, { input: JSON.stringify(variables) });
  const result = JSON.parse(execFileSync('railway', args, {
    input: JSON.stringify(variables), encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'],
  }));
  if (result.errors?.length) throw new Error('Railway GraphQL request failed.');
  return result;
}

async function main() {
  const [command, file, ...args] = process.argv.slice(2);
  if (command === 'image-repository') {
    const manifest = JSON.parse(readFileSync(process.env.SCOPE_DEPLOYMENT_MANIFEST || '.github/deployment-services.json', 'utf8'));
    console.log(releaseImageRepository(manifest, process.env.GITHUB_REPOSITORY, file));
  } else if (command === 'verify-private-package') {
    const manifest = JSON.parse(readFileSync(process.env.SCOPE_DEPLOYMENT_MANIFEST || '.github/deployment-services.json', 'utf8'));
    await verifyPrivateReleasePackage(manifest, process.env.GITHUB_REPOSITORY, file, { token: process.env.GITHUB_TOKEN });
  } else if (command === 'configure-staging-registry') {
    const manifest = JSON.parse(readFileSync(process.env.SCOPE_DEPLOYMENT_MANIFEST || '.github/deployment-services.json', 'utf8'));
    configureStagingRegistry(manifest, { username: process.env.SCOPE_RAILWAY_REGISTRY_USERNAME, password: process.env.SCOPE_RAILWAY_REGISTRY_PASSWORD });
  } else if (command === 'record') {
    const [component, image, sourceSha, serviceId] = args;
    let release;
    try { release = JSON.parse(readFileSync(file, 'utf8')); } catch (error) {
      if (error.code !== 'ENOENT') throw error;
      release = { schemaVersion: 1, sourceSha, components: {} };
      if (process.env.GITHUB_RUN_ID) release.preparationRunId = process.env.GITHUB_RUN_ID;
      if (process.env.SCOPE_MAINTENANCE_BINARY) release.maintenanceSha256 = createHash('sha256').update(readFileSync(process.env.SCOPE_MAINTENANCE_BINARY)).digest('hex');
    }
    validatePreparedRelease(release, { sourceSha });
    release.components[component] = { sourceSha, image, serviceId };
    validatePreparedRelease(release);
    writeFileSync(`${file}.tmp`, `${JSON.stringify(release, null, 2)}\n`);
    renameSync(`${file}.tmp`, file);
  } else {
    const release = JSON.parse(readFileSync(file, 'utf8'));
    const services = JSON.parse(readFileSync(process.env.SCOPE_DEPLOYMENT_MANIFEST || '.github/deployment-services.json', 'utf8')).services;
    if (command === 'validate') {
      const [sourceSha, ...components] = args;
      validatePreparedRelease(release, { sourceSha, components, services });
      console.log(JSON.stringify(release));
    } else if (command === 'verify-maintenance') {
      const [binaryFile] = args;
      console.log(validateMaintenanceArtifact(release, readFileSync(binaryFile)));
    } else if (command === 'verify') {
      const [component, deploymentFile, deploymentId] = args;
      const value = JSON.parse(readFileSync(deploymentFile, 'utf8'));
      const deployment = Array.isArray(value) ? value.find((entry) => entry.id === deploymentId) : value;
      validatePreparedRelease(release, { components: [component], services });
      assertActivatedArtifact(release, component, deployment, { deploymentId });
      console.log(JSON.stringify({ deploymentId, image: release.components[component].image, sourceSha: release.sourceSha }));
    } else if (command === 'activate') {
      const [component, environmentId] = args;
      const config = JSON.parse(readFileSync(`${componentPaths[component]}/railway.json`, 'utf8'));
      const username = process.env.SCOPE_RAILWAY_REGISTRY_USERNAME;
      const password = process.env.SCOPE_RAILWAY_REGISTRY_PASSWORD;
      const registryCredentials = username || password ? { username, password } : undefined;
      console.log(JSON.stringify(activateArtifact(release, component, environmentId, { config, services, registryCredentials })));
    } else throw new Error('usage: railway-artifact.mjs <validate|record|activate|verify|verify-maintenance|configure-staging-registry|image-repository|verify-private-package> <manifest> <arguments...>');
  }
}
if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => { console.error(error.message); process.exitCode = 1; });
}
