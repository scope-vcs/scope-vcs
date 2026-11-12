#!/usr/bin/env node
import { pathToFileURL } from 'node:url';

function storedReplicaRegions(environmentConfig, serviceId) {
  return Object.entries(environmentConfig.services?.[serviceId]?.deploy?.multiRegionConfig || {})
    .filter(([, config]) => config && Number(config.numReplicas) > 0);
}

function storedInSingleRegion(environmentConfig, serviceId, region, replicas) {
  const stored = storedReplicaRegions(environmentConfig, serviceId);
  return stored.length === 1 && stored[0][0] === region && Number(stored[0][1].numReplicas) === replicas;
}

export function productionTargetFromEnvironment(env = process.env) {
  return {
    projectId: env.RAILWAY_PROJECT_ID,
    environmentId: env.SCOPE_RAILWAY_ENVIRONMENT_ID,
    routerGroupId: env.SCOPE_RAILWAY_ROUTER_GROUP_ID,
    services: {
      api: { id: env.SCOPE_RAILWAY_API_SERVICE_ID, name: 'scope-api', region: env.SCOPE_RAILWAY_API_REGION_ID },
      'run-worker': { id: env.SCOPE_RAILWAY_WORKER_SERVICE_ID, name: 'scope-run-worker', region: env.SCOPE_RAILWAY_WORKER_REGION_ID },
      'git-router': { id: env.SCOPE_RAILWAY_ROUTER_SERVICE_ID, name: 'scope-git-router' },
      'media-api': { id: env.SCOPE_RAILWAY_MEDIA_SERVICE_ID, name: 'scope-media-api', region: env.SCOPE_RAILWAY_MEDIA_REGION_ID },
      'media-worker': { id: env.SCOPE_RAILWAY_MEDIA_WORKER_SERVICE_ID, name: 'scope-media-worker', region: env.SCOPE_RAILWAY_MEDIA_REGION_ID },
      database: { id: env.SCOPE_RAILWAY_DATABASE_SERVICE_ID, name: 'scope-postgres' },
    },
  };
}

export function verifyProductionTarget({ status = {}, services = [], environmentConfig = {} }, target) {
  const fail = (message) => { throw new Error(`Refusing backend deployment: ${message}.`); };
  const environments = status.environments?.edges?.map(({ node }) => node) || [];
  const declared = status.services?.edges?.map(({ node }) => node) || [];
  if (status.id !== target.projectId) fail('Railway project ID does not match the reviewed target');
  if (!environments.some(({ id, name }) => id === target.environmentId && name === 'production')) {
    fail('Railway production environment does not match the reviewed target');
  }
  for (const { id, name } of Object.values(target.services)) {
    if (!declared.some((service) => service.id === id && service.name === name)) {
      fail(`Railway service ${name} does not match the reviewed target`);
    }
  }
  for (const { id, name, region } of ['api', 'run-worker', 'media-api', 'media-worker'].map((key) => target.services[key])) {
    const service = services.find((candidate) => candidate.id === id);
    if (!service) fail(`Railway service ${name} has no production state`);
    const configured = service.replicas?.configured || 0;
    if (configured > 0 && !service.regions?.some((live) => live.name === region && live.configured === configured)) {
      fail(`Railway service ${name} is not running in reviewed region ${region}`);
    }
    if (configured > 0 && !storedInSingleRegion(environmentConfig, id, region, configured)) {
      fail(`Railway service ${name} stored region config does not match reviewed region ${region}`);
    }
  }
}

export function verifyRouterTopology({ services = [], environmentConfig = {} }, target) {
  const { id } = target.services['git-router'];
  const region = target.services.api.region;
  const router = services.find((service) => service.id === id);
  const replicas = router?.replicas || {};
  if (environmentConfig.services?.[id]?.groupId !== target.routerGroupId ||
      router?.status !== 'SUCCESS' || router.deploymentStopped === true ||
      replicas.configured !== 1 || replicas.running !== 1 || (replicas.crashed || 0) !== 0 ||
      router.regions?.find(({ name }) => name === region)?.configured !== 1 ||
      !storedInSingleRegion(environmentConfig, id, region, 1)) {
    throw new Error('Production router topology does not match the reviewed region and replica count.');
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const json = (name, fallback) => JSON.parse(process.env[name] || fallback);
    const state = {
      status: json('RAILWAY_STATUS_JSON', '{}'),
      services: json('RAILWAY_SERVICES_JSON', '[]'),
      environmentConfig: json('RAILWAY_ENVIRONMENT_CONFIG_JSON', '{}'),
    };
    const verify = { production: verifyProductionTarget, router: verifyRouterTopology }[process.argv[2]];
    if (!verify) throw new Error('usage: railway-production-target.mjs production|router');
    verify(state, productionTargetFromEnvironment());
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
