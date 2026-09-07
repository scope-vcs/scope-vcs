#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { generateKeyPairSync, randomBytes } from "node:crypto";
import { readFileSync, writeFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

export const RESOURCE_NAMES = Object.freeze({
  bucket: "scope-request-media",
  gateway: "scope-media",
  worker: "scope-media-worker",
});

const IMAGE_DIGEST = /^ghcr\.io\/scope-vcs\/scope-media-worker@sha256:[0-9a-f]{64}$/;
const HOSTNAME = /^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?(?:\.[a-z0-9](?:[a-z0-9-]*[a-z0-9])?)+$/;
const SECRET_VARIABLES = Object.freeze({
  api: ["SCOPE_MEDIA_GRANT_PRIVATE_KEY"],
  gateway: ["SCOPE_MEDIA_ENCRYPTION_KEY"],
  worker: ["SCOPE_MEDIA_ENCRYPTION_KEY"],
});
const STAGING_KEY_VARIABLES = Object.freeze([
  ["api", "SCOPE_MEDIA_GRANT_PRIVATE_KEY"],
  ["gateway", "SCOPE_MEDIA_GRANT_PUBLIC_KEY"],
  ["gateway", "SCOPE_MEDIA_ENCRYPTION_KEY"],
  ["worker", "SCOPE_MEDIA_ENCRYPTION_KEY"],
]);
const TOPOLOGY_ACTIONS = new Set([
  "attachBucket",
  "attachService",
  "createBucket",
  "createService",
]);

function requiredString(value, label) {
  if (typeof value !== "string" || value.length === 0) throw new Error(`${label} is required`);
  return value;
}

function oneNamed(items, name, kind) {
  const matches = items.filter((item) => item.name === name);
  if (matches.length > 1) throw new Error(`Multiple Railway ${kind}s are named ${name}`);
  return matches[0] ?? null;
}

function serviceVariables(bucketName) {
  return {
    DATABASE_URL: "${{scope-postgres.DATABASE_URL}}",
    SCOPE_MEDIA_BUCKET_NAME: `\${{${bucketName}.BUCKET}}`,
    SCOPE_MEDIA_BUCKET_ENDPOINT: `\${{${bucketName}.ENDPOINT}}`,
    SCOPE_MEDIA_BUCKET_ACCESS_KEY_ID: `\${{${bucketName}.ACCESS_KEY_ID}}`,
    SCOPE_MEDIA_BUCKET_SECRET_ACCESS_KEY: `\${{${bucketName}.SECRET_ACCESS_KEY}}`,
    SCOPE_MEDIA_BUCKET_REGION: `\${{${bucketName}.REGION}}`,
  };
}

export function desiredMediaState(manifest, environmentName, workerImageDigest = "") {
  const railway = manifest?.railway;
  const resources = manifest?.mediaResources;
  if (!railway || !resources) throw new Error("Deployment manifest has no media resource contract");
  const environmentId = environmentName === "production"
    ? railway.environmentId
    : environmentName === railway.staging?.environmentName
      ? railway.staging.environmentId
      : "";
  requiredString(environmentId, `Railway ${environmentName} environment ID`);
  const bucketName = requiredString(resources.bucket?.name, "media bucket name");
  const allowedOrigin = environmentName === "production"
    ? requiredString(resources.production?.webOrigin, "production web origin")
    : `https://${requiredString(railway.staging?.webDomain, "staging web domain")}`;
  const image = workerImageDigest || resources.workerImageDigest || "";
  if (image && !IMAGE_DIGEST.test(image)) {
    throw new Error("Media worker image must be the reviewed GHCR repository pinned by sha256 digest");
  }

  return {
    projectId: requiredString(railway.projectId, "Railway project ID"),
    environmentId,
    environmentName,
    regionId: requiredString(railway.regionId, "Railway region ID"),
    bucket: {
      id: resources.bucket.id ?? null,
      name: bucketName,
      region: requiredString(resources.bucket.region, "media bucket region"),
    },
    services: {
      api: {
        id: manifest.services?.api?.id ?? null,
        name: requiredString(manifest.services?.api?.name, "API service name"),
        variables: {
          SCOPE_MEDIA_PUBLIC_URL: "https://${{scope-media.RAILWAY_PUBLIC_DOMAIN}}",
        },
      },
      gateway: {
        id: manifest.services?.media?.id ?? null,
        name: RESOURCE_NAMES.gateway,
        variables: {
          ...serviceVariables(bucketName),
          SCOPE_MEDIA_GRANT_PUBLIC_KEY: null,
          SCOPE_MEDIA_ALLOWED_ORIGIN: allowedOrigin,
        },
        config: {
          healthcheckPath: "/readyz",
          healthcheckTimeout: 60,
          multiRegionConfig: { [railway.regionId]: { numReplicas: 1 } },
          restartPolicyMaxRetries: 10,
          restartPolicyType: "ON_FAILURE",
        },
        publicDomain: true,
      },
      worker: {
        id: manifest.services?.mediaWorker?.id ?? null,
        name: RESOURCE_NAMES.worker,
        variables: serviceVariables(bucketName),
        config: {
          healthcheckPath: "/healthz",
          healthcheckTimeout: 60,
          multiRegionConfig: { [railway.regionId]: { numReplicas: 1 } },
          restartPolicyMaxRetries: 10,
          restartPolicyType: "ON_FAILURE",
          ...(image ? { source: { image } } : {}),
        },
        publicDomain: false,
      },
    },
    expectedManifestDomain: resources[environmentName === "production" ? "production" : "staging"]
      ?.gatewayDomain ?? null,
  };
}

function sameJson(left, right) {
  return JSON.stringify(left) === JSON.stringify(right);
}

export function planMediaReconcile(desired, current, { requireManifestIds = false } = {}) {
  if (current.projectId !== desired.projectId) throw new Error("Railway project does not match the manifest");
  if (current.environmentId !== desired.environmentId || current.environmentName !== desired.environmentName) {
    throw new Error("Railway environment does not match the requested manifest target");
  }
  const operations = [];
  const blockers = [];
  const manualActions = [];
  const projectBucket = oneNamed(current.projectBuckets ?? current.buckets ?? [], desired.bucket.name, "project bucket");
  const bucket = oneNamed(current.buckets ?? [], desired.bucket.name, "bucket instance");
  if (projectBucket) {
    if (desired.bucket.id && projectBucket.id !== desired.bucket.id) {
      blockers.push(`project bucket ID differs: ${projectBucket.id}`);
    }
    if (!bucket) {
      operations.push({
        action: "attachBucket",
        id: projectBucket.id,
        name: desired.bucket.name,
        region: desired.bucket.region,
      });
    } else if (bucket.id !== projectBucket.id) {
      blockers.push(`bucket instance ID differs from project bucket: ${bucket.id}`);
    }
  } else if (desired.bucket.id) {
    blockers.push(`manifest bucket ${desired.bucket.id} is absent from the Railway project`);
  } else {
    operations.push({ action: "createBucket", name: desired.bucket.name, region: desired.bucket.region });
  }

  for (const [role, wanted] of Object.entries(desired.services)) {
    const projectService = oneNamed(
      current.projectServices ?? current.services ?? [],
      wanted.name,
      "project service",
    );
    const actual = oneNamed(current.services ?? [], wanted.name, "service instance");
    if (projectService && wanted.id && projectService.id !== wanted.id) {
      blockers.push(`${wanted.name} project service ID differs: ${projectService.id}`);
    }
    if (!actual) {
      if (role === "api") blockers.push(`required existing service ${wanted.name} is missing`);
      else if (projectService) {
        operations.push({ action: "attachService", role, id: projectService.id, name: wanted.name });
      } else if (wanted.id) {
        blockers.push(`manifest service ${wanted.name} (${wanted.id}) is absent from the Railway project`);
      } else {
        operations.push({ action: "createService", role, name: wanted.name });
      }
      continue;
    }
    if (projectService && actual.id !== projectService.id) {
      blockers.push(`${wanted.name} instance ID differs from project service: ${actual.id}`);
    }
    if (wanted.id && actual.id !== wanted.id) blockers.push(`${wanted.name} ID differs: ${actual.id}`);

    for (const [name, value] of Object.entries(wanted.variables ?? {})) {
      if (value === null) {
        if (!(name in (actual.variables ?? {})) && desired.environmentName === "production") {
          manualActions.push(`set and seal ${wanted.name}.${name}`);
        }
      } else if (actual.variables?.[name] !== value) {
        operations.push({ action: "setVariable", serviceId: actual.id, serviceName: wanted.name, name, value });
      }
    }
    for (const secretName of desired.environmentName === "production" ? SECRET_VARIABLES[role] ?? [] : []) {
      const metadata = (actual.variableMetadata ?? []).find(({ name }) => name === secretName);
      if (!metadata) manualActions.push(`set and seal ${wanted.name}.${secretName}`);
      else if (metadata.isSealed !== true) manualActions.push(`seal ${wanted.name}.${secretName}`);
    }
    if (wanted.config) {
      const actualConfig = actual.config ?? {};
      const drift = Object.fromEntries(Object.entries(wanted.config).filter(([key, value]) => !sameJson(actualConfig[key], value)));
      if (Object.keys(drift).length > 0) {
        if (drift.source && !desired.services.worker.config.source) blockers.push("worker image digest is required");
        else operations.push({ action: "configureService", serviceId: actual.id, serviceName: wanted.name, input: drift });
      }
    }
    const serviceDomains = (actual.domains ?? []).filter(({ type }) => type === "service");
    if (wanted.publicDomain && serviceDomains.length === 0) {
      operations.push({ action: "createDomain", serviceId: actual.id, serviceName: wanted.name, targetPort: 8080 });
    }
    if (wanted.publicDomain === false && serviceDomains.length > 0) {
      blockers.push(`${wanted.name} must remain private and has a public service domain`);
    }
  }

  if (desired.environmentName === "staging") {
    const keyedServices = Object.fromEntries(Object.entries(desired.services).map(([role, wanted]) => [
      role,
      oneNamed(current.services ?? [], wanted.name, "service"),
    ]));
    if (STAGING_KEY_VARIABLES.every(([role]) => keyedServices[role])) {
      const present = STAGING_KEY_VARIABLES.filter(([role, name]) => (
        name in (keyedServices[role].variables ?? {})
      ));
      if (present.length === 0) {
        operations.push({
          action: "generateStagingSecrets",
          services: Object.fromEntries(Object.entries(keyedServices).map(([role, service]) => [role, service.id])),
        });
      } else if (present.length !== STAGING_KEY_VARIABLES.length) {
        blockers.push("staging media key set is incomplete; remove the partial key set before retrying");
      }
    }
  }

  const gateway = oneNamed(current.services ?? [], desired.services.gateway.name, "service");
  const liveDomain = gateway?.domains?.find(({ type }) => type === "service")?.domain ?? null;
  if (liveDomain && !HOSTNAME.test(liveDomain)) blockers.push("gateway domain is invalid");
  if (desired.expectedManifestDomain && liveDomain && desired.expectedManifestDomain !== liveDomain) {
    blockers.push(`gateway domain differs from manifest: ${liveDomain}`);
  }
  if (requireManifestIds) {
    if (!desired.bucket.id) blockers.push("manifest media bucket ID is not recorded");
    if (!desired.services.gateway.id) blockers.push("manifest media gateway service ID is not recorded");
    if (!desired.services.worker.id) blockers.push("manifest media worker service ID is not recorded");
    if (!desired.expectedManifestDomain) blockers.push("manifest media gateway domain is not recorded");
  }
  return {
    blockers: [...new Set(blockers)].sort(),
    manualActions: [...new Set(manualActions)].sort(),
    liveDomain,
    operations,
  };
}

export function normalizeLiveServiceConfig(serviceConfig = {}) {
  const deploy = serviceConfig.deploy ?? {};
  return {
    ...deploy,
    restartPolicyMaxRetries: deploy.restartPolicyMaxRetries ?? 10,
    restartPolicyType: deploy.restartPolicyType ?? "ON_FAILURE",
    ...(serviceConfig.source ? { source: serviceConfig.source } : {}),
  };
}

export function normalizeLiveVariables(variables = {}) {
  return Object.fromEntries(Object.entries(variables).map(([name, entry]) => [
    name,
    entry && typeof entry === "object" && "value" in entry ? entry.value : entry,
  ]));
}

export async function generateStagingSecrets(serviceIds, setVariable) {
  const { privateKey, publicKey } = generateKeyPairSync("ed25519", {
    privateKeyEncoding: { type: "pkcs8", format: "pem" },
    publicKeyEncoding: { type: "spki", format: "pem" },
  });
  const encryptionKey = randomBytes(32).toString("base64");
  await setVariable(serviceIds.api, "SCOPE_MEDIA_GRANT_PRIVATE_KEY", privateKey);
  await setVariable(serviceIds.gateway, "SCOPE_MEDIA_GRANT_PUBLIC_KEY", publicKey);
  await setVariable(serviceIds.gateway, "SCOPE_MEDIA_ENCRYPTION_KEY", encryptionKey);
  await setVariable(serviceIds.worker, "SCOPE_MEDIA_ENCRYPTION_KEY", encryptionKey);
}

export function assertConfigurationReady(plan) {
  if (plan.blockers.length > 0) {
    throw new Error(`Refusing configuration because Railway topology drift remains: ${plan.blockers.join("; ")}`);
  }
  const remaining = plan.operations.filter(({ action }) => TOPOLOGY_ACTIONS.has(action));
  if (remaining.length > 0) {
    throw new Error(`Refusing configuration because Railway topology operations remain: ${remaining.map(({ action, name }) => `${action} ${name}`).join("; ")}`);
  }
}

export function configurationCanApply(plan) {
  return plan.manualActions.length === 0;
}

export async function applyCreationOperations(operations, adapter) {
  const created = [];
  try {
    for (const operation of operations.filter(({ action }) => TOPOLOGY_ACTIONS.has(action))) {
      const resource = await adapter[operation.action](operation);
      created.push({ ...operation, id: requiredString(resource?.id, `${operation.name} created resource ID`) });
    }
    return created;
  } catch (error) {
    for (const resource of created.reverse()) {
      try {
        if (resource.action === "createBucket") await adapter.deleteBucket(resource);
        else if (resource.action === "createService") await adapter.deleteService(resource);
        else if (resource.action === "attachBucket") await adapter.detachBucket(resource);
        else await adapter.detachService(resource);
      } catch (rollbackError) {
        error.message += `; rollback failed for ${resource.name}: ${rollbackError.message}`;
      }
    }
    throw error;
  }
}

function railway(args, options = {}) {
  return execFileSync("railway", args, { encoding: "utf8", stdio: ["ignore", "pipe", "inherit"], ...options });
}

function railwayJson(args) {
  return JSON.parse(railway(args));
}

function setRailwayVariable(desired, serviceId, name, value) {
  execFileSync("railway", [
    "variable", "set", "--project", desired.projectId, "--environment", desired.environmentId,
    "--service", serviceId, "--skip-deploys", "--stdin", name,
  ], { input: value, encoding: "utf8", stdio: ["pipe", "ignore", "inherit"] });
}

function assertLinkedProject(projectId, environmentId) {
  const status = railwayJson(["status", "--environment", environmentId, "--json"]);
  if (status.id !== projectId) {
    throw new Error(`Linked Railway project ${status.id ?? "unknown"} does not match ${projectId}`);
  }
  return status;
}

function variableMetadata(projectId, environmentId) {
  const query = `query MediaVariables($environmentId: String!) {
    environment(id: $environmentId) {
      variables(first: 500) { edges { node { name serviceId isSealed } } }
    }
  }`;
  const response = railwayJson(["api", query, "--variables", JSON.stringify({ environmentId }), "--compact"]);
  return response.data?.environment?.variables?.edges?.map(({ node }) => node) ?? [];
}

export function loadLiveMediaState(desired) {
  const status = assertLinkedProject(desired.projectId, desired.environmentId);
  const services = railwayJson([
    "service", "list", "--project", desired.projectId, "--environment", desired.environmentId, "--json",
  ]);
  const config = railwayJson(["environment", "config", "--environment", desired.environmentId, "--json"]);
  const metadata = variableMetadata(desired.projectId, desired.environmentId);
  const project = graphql(`query MediaProjectResources($projectId: String!) {
    project(id: $projectId) {
      buckets(first: 500) { edges { node { id name } } }
      services(first: 500) { edges { node { id name } } }
    }
  }`, { projectId: desired.projectId }).project;
  for (const service of services) {
    if (!Object.values(desired.services).some(({ name }) => name === service.name)) continue;
    const serviceConfig = config.services?.[service.id] ?? {};
    service.variables = normalizeLiveVariables(serviceConfig.variables);
    service.variableMetadata = metadata.filter(({ serviceId }) => serviceId === service.id);
    const domains = railwayJson([
      "domain", "list", "--project", desired.projectId, "--environment", desired.environmentId,
      "--service", service.id, "--json",
    ]);
    service.domains = domains.domains ?? domains;
    service.config = normalizeLiveServiceConfig(serviceConfig);
  }
  return {
    projectId: status.id,
    environmentId: desired.environmentId,
    environmentName: status.environments?.edges?.map(({ node }) => node)
      .find(({ id }) => id === desired.environmentId)?.name,
    projectBuckets: project?.buckets?.edges?.map(({ node }) => node) ?? [],
    projectServices: project?.services?.edges?.map(({ node }) => node) ?? [],
    buckets: railwayJson(["bucket", "list", "--environment", desired.environmentId, "--json"]),
    services,
  };
}

function graphql(query, variables) {
  const response = railwayJson(["api", query, "--variables", JSON.stringify(variables), "--compact"]);
  if (response.errors?.length) throw new Error(response.errors.map(({ message }) => message).join("; "));
  return response.data;
}

function liveAdapter(desired) {
  const patchEnvironment = (patch, commitMessage) => graphql(
    `mutation PatchMediaEnvironment($environmentId: String!, $patch: EnvironmentConfig!, $commitMessage: String) {
      environmentPatchCommit(environmentId: $environmentId, patch: $patch, commitMessage: $commitMessage)
    }`,
    { environmentId: desired.environmentId, patch, commitMessage },
  );
  return {
    async createBucket({ name, region }) {
      assertLinkedProject(desired.projectId, desired.environmentId);
      const result = railwayJson(["bucket", "create", name, "--region", region, "--environment", desired.environmentId, "--json"]);
      return { id: result.id ?? result.bucketId };
    },
    async createService({ name }) {
      assertLinkedProject(desired.projectId, desired.environmentId);
      return graphql(`mutation CreateMediaService($input: ServiceCreateInput!) {
        serviceCreate(input: $input) { id }
      }`, { input: {
        projectId: desired.projectId,
        environmentId: desired.environmentId,
        name,
      } }).serviceCreate;
    },
    async attachBucket({ id, name, region }) {
      assertLinkedProject(desired.projectId, desired.environmentId);
      patchEnvironment({ buckets: { [id]: { isCreated: true, region } } }, `Attach ${name} bucket`);
      return { id };
    },
    async attachService({ id, name }) {
      assertLinkedProject(desired.projectId, desired.environmentId);
      patchEnvironment({ services: { [id]: { isCreated: true } } }, `Attach ${name} service`);
      return { id };
    },
    async deleteBucket({ id }) {
      patchEnvironment({ buckets: { [id]: { isDeleted: true } } }, "Roll back media bucket creation");
    },
    async deleteService({ id }) {
      graphql("mutation DeleteMediaService($id: String!) { serviceDelete(id: $id) }", { id });
    },
    async detachBucket({ id }) {
      patchEnvironment({ buckets: { [id]: { isDeleted: true } } }, "Roll back media bucket attachment");
    },
    async detachService({ id }) {
      graphql(
        "mutation DetachMediaService($id: String!, $environmentId: String!) { serviceDelete(id: $id, environmentId: $environmentId) }",
        { id, environmentId: desired.environmentId },
      );
    },
  };
}

async function applyConfigurationOperations(operations, desired) {
  for (const operation of operations) {
    if (operation.action === "setVariable") {
      railway([
        "variable", "set", "--project", desired.projectId, "--environment", desired.environmentId,
        "--service", operation.serviceId, "--skip-deploys", `${operation.name}=${operation.value}`, "--json",
      ]);
    } else if (operation.action === "configureService") {
      graphql(`mutation ConfigureMediaService($serviceId: String!, $environmentId: String!, $input: ServiceInstanceUpdateInput!) {
        serviceInstanceUpdate(serviceId: $serviceId, environmentId: $environmentId, input: $input)
      }`, { serviceId: operation.serviceId, environmentId: desired.environmentId, input: operation.input });
    } else if (operation.action === "createDomain") {
      graphql(`mutation CreateMediaDomain($input: ServiceDomainCreateInput!) {
        serviceDomainCreate(input: $input) { id domain }
      }`, { input: { environmentId: desired.environmentId, serviceId: operation.serviceId, targetPort: operation.targetPort } });
    } else if (operation.action === "generateStagingSecrets") {
      await generateStagingSecrets(operation.services, (serviceId, name, value) => {
        setRailwayVariable(desired, serviceId, name, value);
      });
    }
  }
}

function updateManifest(path, manifest, desired, current) {
  const bucket = oneNamed(current.buckets, desired.bucket.name, "bucket");
  const gateway = oneNamed(current.services, desired.services.gateway.name, "service");
  const worker = oneNamed(current.services, desired.services.worker.name, "service");
  const domain = gateway?.domains?.find(({ type }) => type === "service")?.domain;
  if (!bucket || !gateway || !worker || !domain) throw new Error("Cannot record incomplete media resources");
  manifest.mediaResources.bucket.id = bucket.id;
  manifest.services.media.id = gateway.id;
  manifest.services.mediaWorker.id = worker.id;
  const key = desired.environmentName === "production" ? "production" : "staging";
  manifest.mediaResources[key].gatewayDomain = domain;
  writeFileSync(path, `${JSON.stringify(manifest, null, 2)}\n`);
}

function argument(name, fallback = "") {
  const index = process.argv.indexOf(name);
  return index === -1 ? fallback : process.argv[index + 1] ?? fallback;
}

async function main() {
  const action = process.argv[2];
  if (!["plan", "apply", "verify"].includes(action)) {
    throw new Error("usage: reconcile-media.mjs <plan|apply|verify> --environment <staging|production> [--worker-image <digest>] [--write-manifest]");
  }
  const manifestPath = argument("--manifest", ".github/deployment-services.json");
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  const desired = desiredMediaState(manifest, argument("--environment"), argument("--worker-image"));
  let current = loadLiveMediaState(desired);
  let plan = planMediaReconcile(desired, current, { requireManifestIds: action === "verify" });

  if (action === "apply") {
    if (plan.blockers.length > 0) {
      throw new Error(`Refusing mutation because the Railway target has identity or topology drift: ${plan.blockers.join("; ")}`);
    }
    const created = await applyCreationOperations(plan.operations, liveAdapter(desired));
    try {
      if (created.length > 0) current = loadLiveMediaState(desired);
      plan = planMediaReconcile(desired, current);
      assertConfigurationReady(plan);
      if (configurationCanApply(plan)) {
        await applyConfigurationOperations(plan.operations, desired);
        current = loadLiveMediaState(desired);
        plan = planMediaReconcile(desired, current);
      }
    } catch (error) {
      const adapter = liveAdapter(desired);
      for (const resource of created.reverse()) {
        try {
          if (resource.action === "createBucket") await adapter.deleteBucket(resource);
          else if (resource.action === "createService") await adapter.deleteService(resource);
          else if (resource.action === "attachBucket") await adapter.detachBucket(resource);
          else await adapter.detachService(resource);
        } catch (rollbackError) {
          error.message += `; rollback failed for ${resource.name}: ${rollbackError.message}`;
        }
      }
      throw error;
    }
    if (process.argv.includes("--write-manifest") && plan.operations.length === 0 && plan.blockers.length === 0) {
      updateManifest(manifestPath, manifest, desired, current);
    }
  }

  process.stdout.write(`${JSON.stringify({
    action,
    environment: desired.environmentName,
    operations: plan.operations,
    blockers: plan.blockers,
    manualActions: plan.manualActions,
    discovered: {
      bucketId: oneNamed(current.buckets ?? [], desired.bucket.name, "bucket")?.id ?? null,
      gatewayServiceId: oneNamed(current.services ?? [], desired.services.gateway.name, "service")?.id ?? null,
      workerServiceId: oneNamed(current.services ?? [], desired.services.worker.name, "service")?.id ?? null,
      gatewayDomain: plan.liveDomain,
    },
  }, null, 2)}\n`);
  if (plan.blockers.length > 0 || plan.manualActions.length > 0 || (action === "verify" && plan.operations.length > 0)) {
    process.exitCode = 1;
  }
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
}
