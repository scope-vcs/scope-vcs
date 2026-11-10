#!/usr/bin/env node

import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

import { previewEnvironmentPullRequest } from "./preview-environment-identity.mjs";
import { qualifiesForPreview } from "./preview-pull-request.mjs";
import { railwayEnvironmentInventory } from "./railway-environment-inventory.mjs";

const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
function utcTimestamp(value) {
  if (typeof value !== "string" || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{3})?Z$/.test(value)) return null;
  const milliseconds = Date.parse(value);
  return Number.isFinite(milliseconds) && new Date(milliseconds).toISOString() === (value.includes(".") ? value : value.replace("Z", ".000Z"))
    ? milliseconds : null;
}

export function auditRailwayExperiments({ manifest, registry, project, repository, pullRequests = [], now = Date.now() }) {
  const issues = [];
  if (!Number.isFinite(now)) throw new Error("Audit time must be valid");
  const { environments, protectedIds, liveIds } = railwayEnvironmentInventory(manifest, project);
  if (!registry || typeof registry !== "object" || Array.isArray(registry)) throw new Error("Experiment registry must be an object keyed by provider environment ID");
  for (const id of protectedIds) if (!liveIds.has(id)) issues.push(`Protected environment ${id} is missing`);
  for (const [id, entry] of Object.entries(registry)) {
    if (!uuid.test(id)) issues.push(`Registry key ${id} is not an exact provider environment ID`);
    if (protectedIds.includes(id)) issues.push(`Protected environment ${id} must not be registered as an experiment`);
    if (!liveIds.has(id)) issues.push(`Registry entry ${id} has no live environment; remove its stale registration`);
    if (typeof entry?.owner !== "string" || !entry.owner.trim()) issues.push(`Experiment ${id} requires an owner`);
    const expiry = utcTimestamp(entry?.expiresAt);
    if (expiry === null) issues.push(`Experiment ${id} requires a valid UTC expiresAt timestamp`);
    else if (expiry <= now) issues.push(`Experiment ${id} expired at ${entry.expiresAt}; review its data and retirement`);
  }
  const experiments = environments.filter(environment => {
    if (protectedIds.includes(environment.id)) return false;
    const number = previewEnvironmentPullRequest(manifest, environment);
    return !(number !== null && pullRequests.some(pr => pr.number === number && qualifiesForPreview(pr, repository)));
  });
  for (const environment of experiments) {
    const match = /^test-[a-z0-9]+(?:-[a-z0-9]+)*-(\d{4})(\d{2})(\d{2})$/.exec(environment.name ?? "");
    const date = match ? `${match[1]}-${match[2]}-${match[3]}` : "";
    if (!match || utcTimestamp(`${date}T00:00:00Z`) === null) issues.push(`Environment ${environment.id} must be named test-<purpose>-<yyyymmdd> with a valid date`);
    if (!Object.hasOwn(registry, environment.id)) issues.push(`Environment ${environment.id} is unregistered`);
  }
  return { ok: issues.length === 0, protectedEnvironmentCount: protectedIds.length,
    experimentCount: experiments.length, issues };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    const json = path => JSON.parse(readFileSync(path, "utf8"));
    const result = auditRailwayExperiments({ manifest: json(".github/deployment-services.json"),
      registry: json(".github/railway-experiments.json"), project: json(process.argv[2]).data?.project,
      repository: process.env.GITHUB_REPOSITORY, pullRequests: json(process.argv[3]) });
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
    if (!result.ok) process.exitCode = 1;
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
