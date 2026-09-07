#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { appendFileSync } from "node:fs";

const [serviceId, image] = process.argv.slice(2);
const projectId = process.env.RAILWAY_PROJECT_ID;
const environmentId = process.env.SCOPE_RAILWAY_ENVIRONMENT_ID;
const component = process.env.SCOPE_DEPLOYMENT_COMPONENT;
const sourceSha = process.env.SCOPE_DEPLOYMENT_SOURCE_SHA || process.env.GITHUB_SHA;
const evidencePath = process.env.SCOPE_DEPLOYMENT_EVIDENCE_PATH;
const mutationToken = process.env.RAILWAY_API_TOKEN || process.env.RAILWAY_TOKEN;
const digestPattern = /^ghcr\.io\/scope-vcs\/scope-media-worker@(sha256:[0-9a-f]{64})$/;
const match = image?.match(digestPattern);

if (!serviceId || !projectId || !environmentId || !component || !sourceSha || !evidencePath || !mutationToken || !match) {
  throw new Error("Digest-pinned Railway image deployment requires service, project, environment, component, source SHA, evidence path, API token, and reviewed GHCR digest");
}

function railway(args, env = process.env) {
  return execFileSync("railway", args, { encoding: "utf8", env });
}

function deployments() {
  return JSON.parse(railway([
    "deployment", "list",
    "--project", projectId,
    "--environment", environmentId,
    "--service", serviceId,
    "--limit", "20",
    "--json",
  ]));
}

const priorIds = new Set(deployments().map(({ id }) => id));
const query = `mutation DeployPinnedMediaWorker($serviceId: String!, $environmentId: String!, $input: ServiceInstanceUpdateInput!) {
  serviceInstanceUpdate(serviceId: $serviceId, environmentId: $environmentId, input: $input)
}`;
const mutationEnvironment = { ...process.env };
if (process.env.RAILWAY_API_TOKEN) delete mutationEnvironment.RAILWAY_TOKEN;
const response = JSON.parse(railway([
  "api", query,
  "--variables", JSON.stringify({
    serviceId,
    environmentId,
    input: { source: { image } },
  }),
  "--compact",
], mutationEnvironment));
if (response.data?.serviceInstanceUpdate !== true || response.errors?.length) {
  throw new Error(`Railway rejected pinned media worker image: ${response.errors?.map(({ message }) => message).join("; ") || "unknown error"}`);
}

const deadline = Date.now() + Number(process.env.SCOPE_IMAGE_DEPLOY_TIMEOUT_MS || 900_000);
let deployment;
while (Date.now() < deadline) {
  deployment = deployments().find((candidate) => {
    const digest = candidate.meta?.imageDigest;
    return !priorIds.has(candidate.id) && digest === match[1];
  });
  if (deployment?.status === "SUCCESS") break;
  if (["FAILED", "CRASHED", "REMOVED"].includes(deployment?.status)) {
    throw new Error(`Railway media worker deployment ${deployment.id} is ${deployment.status}`);
  }
  await new Promise((resolve) => setTimeout(resolve, 10_000));
}
if (deployment?.status !== "SUCCESS") throw new Error("Timed out waiting for the exact media worker image deployment");

appendFileSync(evidencePath, `${JSON.stringify({
  component,
  sourceSha,
  provider: "railway",
  evidenceId: deployment.id,
  artifactDigest: match[1],
})}\n`);
