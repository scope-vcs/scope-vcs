#!/usr/bin/env node

import { RAILWAY_MUTATION_TIMEOUT_MS, retryRailway } from "./railway-retry.mjs";
import { readRailway } from "./railway-read.mjs";
import { execFileSync } from "node:child_process";
import { appendFileSync, readFileSync, renameSync, writeFileSync } from "node:fs";

const projectId = process.env.RAILWAY_PROJECT_ID;
const environmentId = process.env.SCOPE_RAILWAY_ENVIRONMENT_ID;
const mutationToken = process.env.RAILWAY_API_TOKEN || process.env.RAILWAY_TOKEN;
const railwayEnvironment = { ...process.env };
if (process.env.RAILWAY_API_TOKEN) delete railwayEnvironment.RAILWAY_TOKEN;

function railway(args, { input, timeout = RAILWAY_MUTATION_TIMEOUT_MS } = {}) {
  return execFileSync("railway", args, {
    encoding: "utf8",
    timeout, killSignal: "SIGKILL",
    env: railwayEnvironment,
    input,
    stdio: [input === undefined ? "ignore" : "pipe", "pipe", "pipe"],
  });
}

function graphql(query, variables) {
  let response;
  try {
    response = JSON.parse(railway(["api", query, "--variables", "@-", "--compact"], {
      input: JSON.stringify(variables),
    }));
  } catch {
    throw new Error("Railway GraphQL request failed");
  }
  if (response.errors?.length) throw new Error("Railway GraphQL request failed");
  return response.data;
}

function deployments(serviceId) {
  return readRailway([
    "deployment", "list",
    "--project", projectId,
    "--environment", environmentId,
    "--service", serviceId,
    "--limit", "20",
    "--json",
  ], { execute: (_command, args, options) => railway(args, options) });
}

const [first, second, third] = process.argv.slice(2);
if (first === "configure-registry") {
  const serviceId = second;
  const username = process.env.SCOPE_RAILWAY_REGISTRY_USERNAME;
  const password = process.env.SCOPE_RAILWAY_REGISTRY_PASSWORD;
  if (!serviceId || !projectId || !environmentId || !mutationToken || !username || !password) {
    throw new Error("Railway registry configuration requires service, project, environment, API token, username, and password");
  }
  retryRailway(() => {
    const data = graphql(
      "mutation ConfigureRegistry($serviceId:String!,$environmentId:String!,$input:ServiceInstanceUpdateInput!){serviceInstanceUpdate(serviceId:$serviceId,environmentId:$environmentId,input:$input)}",
      { serviceId, environmentId, input: { registryCredentials: { username, password } } },
    );
    if (data?.serviceInstanceUpdate !== true) throw new Error("Railway did not confirm registry configuration");
  });
  process.exit(0);
}

const serviceId = first;
const image = second;
const component = process.env.SCOPE_DEPLOYMENT_COMPONENT;
const sourceSha = process.env.SCOPE_DEPLOYMENT_SOURCE_SHA || process.env.GITHUB_SHA;
const evidencePath = process.env.SCOPE_DEPLOYMENT_EVIDENCE_PATH;
const digestPattern = /^ghcr\.io\/scope-vcs\/scope-media-worker@(sha256:[0-9a-f]{64})$/;
const match = image?.match(digestPattern);

if (!serviceId || third || !projectId || !environmentId || !component || !sourceSha || !evidencePath || !mutationToken || !match) {
  throw new Error("Digest-pinned Railway image deployment requires service, project, environment, component, source SHA, evidence path, API token, and reviewed GHCR digest");
}

const updated = graphql(
  "mutation SelectPinnedImage($serviceId:String!,$environmentId:String!,$input:ServiceInstanceUpdateInput!){serviceInstanceUpdate(serviceId:$serviceId,environmentId:$environmentId,input:$input)}",
  { serviceId, environmentId, input: { source: { image } } },
);
if (updated?.serviceInstanceUpdate !== true) throw new Error("Railway rejected pinned media worker image");

const activated = graphql(
  "mutation DeployPinnedImage($serviceId:String!,$environmentId:String!){serviceInstanceDeployV2(serviceId:$serviceId,environmentId:$environmentId)}",
  { serviceId, environmentId },
);
const deploymentId = activated?.serviceInstanceDeployV2;
if (typeof deploymentId !== "string" || !deploymentId) {
  throw new Error("Railway did not return an exact media worker deployment ID");
}

const deadline = Date.now() + 900_000;
let deployment;
while (Date.now() < deadline) {
  deployment = deployments(serviceId).find((candidate) => candidate.id === deploymentId);
  if (deployment?.status === "SUCCESS") break;
  if (["FAILED", "CRASHED", "REMOVED"].includes(deployment?.status)) {
    throw new Error(`Railway media worker deployment ${deploymentId} is ${deployment.status}`);
  }
  await new Promise((resolve) => setTimeout(resolve, 10_000));
}
if (deployment?.status !== "SUCCESS") {
  throw new Error(`Timed out waiting for Railway media worker deployment ${deploymentId}`);
}
if (deployment.serviceId && deployment.serviceId !== serviceId) {
  throw new Error("Railway returned the exact deployment under a different service");
}
if (deployment.meta?.imageDigest !== match[1]) {
  throw new Error("Successful Railway media worker deployment did not match the reviewed digest");
}

appendFileSync(evidencePath, `${JSON.stringify({
  component,
  sourceSha,
  provider: "railway",
  evidenceId: deploymentId,
  artifactDigest: match[1],
})}\n`);

if (process.env.SCOPE_RELEASE_DEPLOYMENTS_FILE) {
  const deploymentsPath = process.env.SCOPE_RELEASE_DEPLOYMENTS_FILE;
  const active = JSON.parse(readFileSync(deploymentsPath, "utf8"));
  active[component] = deploymentId;
  writeFileSync(`${deploymentsPath}.tmp`, `${JSON.stringify(active)}\n`);
  renameSync(`${deploymentsPath}.tmp`, deploymentsPath);
}
