#!/usr/bin/env node

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";
const REQUIRED_DEPLOY_SETTINGS = [
  "healthcheckPath",
  "healthcheckTimeout",
  "overlapSeconds",
  "drainingSeconds",
];

export function railwayServicesFromStatus(status, environmentId) {
  const environments = status?.environments?.edges?.map(({ node }) => node);
  if (!Array.isArray(environments)) {
    throw new Error("Railway status is missing environments");
  }
  const environment = environments.find(({ id, name }) => (
    id === environmentId || name === environmentId
  ));
  if (!environment) {
    throw new Error(`Railway environment ${environmentId || "unknown"} is missing`);
  }
  const instances = environment.serviceInstances?.edges?.map(({ node }) => node);
  if (!Array.isArray(instances)) {
    throw new Error(`Railway environment ${environmentId} is missing service instances`);
  }
  return instances.map(serviceFromInstance);
}

export function assertEffectiveRailwayDeployConfig(service, railwayConfig, label = "Railway config") {
  const expected = railwayConfig?.deploy;
  if (!expected || typeof expected !== "object") {
    throw new Error(`${label} is missing deploy settings`);
  }
  const effective = service?.effectiveDeploy;
  if (!effective || typeof effective !== "object") {
    throw new Error(
      `Railway service ${service?.name || service?.id || "unknown"} deployment is missing meta.serviceManifest.deploy`,
    );
  }
  for (const setting of REQUIRED_DEPLOY_SETTINGS) {
    if (expected[setting] === undefined || expected[setting] === null) {
      throw new Error(`${label} is missing deploy.${setting}`);
    }
    if (effective[setting] === undefined || effective[setting] === null) {
      throw new Error(
        `Railway service ${service.name || service.id} effective deployment is missing deploy.${setting}`,
      );
    }
    if (String(effective[setting]) !== String(expected[setting])) {
      throw new Error(
        `Railway service ${service.name || service.id} effective deploy.${setting} is ${JSON.stringify(effective[setting])}, expected ${JSON.stringify(expected[setting])}`,
      );
    }
  }
  return service;
}

export function assertHealthyRailwayService(services, serviceId, expectedDeploymentId = "") {
  if (!Array.isArray(services)) throw new Error("Railway service state must be an array");
  const service = services.find(({ id, name }) => id === serviceId || name === serviceId);
  if (!service) throw new Error(`Railway service ${serviceId} is missing`);

  const replicas = railwayReplicaCounts(service);
  if (service.status !== "SUCCESS") {
    throw new Error(`Railway service ${serviceId} is ${service.status || "UNKNOWN"}`);
  }
  if (service.deploymentStopped === true) {
    throw new Error(`Railway service ${serviceId} is stopped`);
  }
  if (!Number.isInteger(replicas.configured) || replicas.configured <= 0) {
    throw new Error(`Railway service ${serviceId} has no configured replicas`);
  }
  if (replicas.running !== replicas.configured) {
    throw new Error(
      `Railway service ${serviceId} has ${replicas.running ?? 0}/${replicas.configured} running replicas`,
    );
  }
  if (replicas.crashed !== 0) {
    throw new Error(`Railway service ${serviceId} has ${replicas.crashed} crashed replicas`);
  }
  if (expectedDeploymentId && service.deploymentId !== expectedDeploymentId) {
    throw new Error(
      `Railway service ${serviceId} is running deployment ${service.deploymentId || "unknown"}, expected ${expectedDeploymentId}`,
    );
  }
  return service;
}

export function railwayReplicaCounts(service) {
  const replicas = service?.replicas;
  for (const key of ["running", "crashed"]) {
    if (!Number.isInteger(replicas?.[key]) || replicas[key] < 0) {
      throw new Error(`Railway service ${service?.name || service?.id || "unknown"} is missing valid ${key} replica evidence`);
    }
  }
  return replicas;
}

export function railwayServiceIsStopped(services, serviceId) {
  if (!Array.isArray(services)) throw new Error("Railway service state must be an array");
  const matches = services.filter(({ id }) => id === serviceId);
  if (matches.length !== 1) throw new Error(`Railway service ${serviceId} must have exactly one state entry`);
  const service = matches[0];
  if (service.status === null && service.deploymentId === null
      && service.latestDeployment === null && service.replicas === null) return true;
  const replicas = railwayReplicaCounts(service);
  return replicas.running === 0 && replicas.crashed === 0;
}

function environmentJson(name) {
  const value = process.env[name];
  if (!value) throw new Error(`${name} is required`);
  return JSON.parse(value);
}

function main() {
  const state = environmentJson("SCOPE_RAILWAY_SERVICES_JSON");
  if (process.argv[2] === "stopped") {
    process.stdout.write(`${railwayServiceIsStopped(state, process.env.SCOPE_RAILWAY_SERVICE_ID)}\n`);
    return;
  }

  const services = servicesFromState(
    state,
    process.env.SCOPE_RAILWAY_ENVIRONMENT_ID ?? "",
  );
  const service = assertHealthyRailwayService(
    services,
    process.env.SCOPE_RAILWAY_SERVICE_ID ?? "",
    process.env.SCOPE_EXPECTED_RAILWAY_DEPLOYMENT_ID ?? "",
  );
  const configPath = process.env.SCOPE_EXPECTED_RAILWAY_CONFIG;
  if (configPath) {
    const config = JSON.parse(readFileSync(resolve(configPath), "utf8"));
    assertEffectiveRailwayDeployConfig(service, config, configPath);
  }
  process.stdout.write(`${JSON.stringify(service)}\n`);
}

function servicesFromState(state, environmentId) {
  return Array.isArray(state)
    ? state
    : railwayServicesFromStatus(state, environmentId);
}

function serviceFromInstance(instance) {
  const deployment = servingDeployment(instance);
  const effectiveDeploy = deployment?.meta?.serviceManifest?.deploy;
  const configured = configuredReplicas(effectiveDeploy, instance.numReplicas);
  const replicaStatuses = deployment?.instances?.map(({ status }) => status) ?? [];
  return {
    id: instance.serviceId,
    name: instance.serviceName,
    status: deployment?.status,
    deploymentId: deployment?.id,
    deploymentStopped: deployment?.deploymentStopped ?? false,
    replicas: deployment ? {
      configured,
      running: replicaStatuses.filter((status) => status === "RUNNING").length,
      crashed: replicaStatuses.filter((status) => status === "CRASHED").length,
    } : undefined,
    effectiveDeploy,
  };
}

function servingDeployment(instance) {
  return instance.activeDeployments?.find(({ status, deploymentStopped }) => (
    status === "SUCCESS" && deploymentStopped !== true
  )) ?? instance.latestDeployment;
}

function configuredReplicas(deploy, fallback) {
  const regions = deploy?.multiRegionConfig;
  if (regions && typeof regions === "object") {
    const total = Object.values(regions).reduce(
      (sum, region) => sum + (Number(region?.numReplicas) || 0),
      0,
    );
    if (total > 0) return total;
  }
  return Number(deploy?.numReplicas ?? fallback);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
