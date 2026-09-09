#!/usr/bin/env node

import { appendFileSync, readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

import { validatePreparedRelease } from "./railway-artifact.mjs";
import { validateRecoveryPreparation } from "./recovery-preparation-trust.mjs";

const components = ["api", "worker", "cache", "router", "media", "mediaWorker", "web"];
const validationJobName = "Production validation gate";

export async function validatePreparedDeployment(
  prepared,
  sourceRunId,
  request,
  repository,
) {
  if (!/^[1-9][0-9]*$/.test(sourceRunId ?? "") || prepared?.preparationRunId !== sourceRunId) {
    throw new Error("Prepared release must match the requested source run ID");
  }

  const manifest = JSON.parse(readFileSync(
    new URL("../deployment-services.json", import.meta.url),
    "utf8",
  ));
  validatePreparedRelease(prepared, {
    services: manifest.services,
    sourceSha: prepared.sourceSha,
  });
  const selected = Object.keys(prepared.components);
  if (selected.length === 0 || selected.some((component) => !components.includes(component))) {
    throw new Error("Prepared deployment requires a nonempty set of application components");
  }
  const backend = selected.some((component) => component !== "web");
  // Backend activation needs the API maintenance tool and the complete backend image set.
  if (backend) validatePreparedRelease(prepared, { components: components.filter((component) => component !== "web") });

  const responses = new Map();
  const cachedRequest = (path) => {
    if (!responses.has(path)) responses.set(path, Promise.resolve(request(path)));
    return responses.get(path);
  };
  const proof = await validateRecoveryPreparation(prepared, cachedRequest, repository, manifest, selected);
  const selection = { backend, ...Object.fromEntries(components.map((component) => [component, selected.includes(component)])) };

  const requiredJobs = new Set([validationJobName, "Prove prepared release in release-proof"]);
  for (let page = 1; ; page += 1) {
    const result = await cachedRequest(
      `/actions/runs/${sourceRunId}/jobs?filter=all&per_page=100&page=${page}`,
    );
    if (!Array.isArray(result.jobs)) throw new Error("Cannot read source validation jobs");
    for (const job of result.jobs) {
      if (String(job.run_id) === sourceRunId
          && job.head_sha === proof.sourceSha
          && job.status === "completed"
          && job.conclusion === "success") requiredJobs.delete(job.name);
    }
    if (requiredJobs.size === 0) return { ...proof, selection };
    if (result.jobs.length < 100) break;
  }
  throw new Error("Source run did not pass the production validation gate and release-proof rehearsal");
}

async function githubRequest(path) {
  const token = process.env.GITHUB_TOKEN;
  const repository = process.env.GITHUB_REPOSITORY;
  if (!token || !repository) throw new Error("GITHUB_TOKEN and GITHUB_REPOSITORY are required");
  const response = await fetch(`https://api.github.com/repos/${repository}${path}`, {
    headers: {
      Accept: "application/vnd.github+json",
      Authorization: `Bearer ${token}`,
      "X-GitHub-Api-Version": "2022-11-28",
    },
    signal: AbortSignal.timeout(15_000),
  });
  if (!response.ok) throw new Error(`GitHub API ${response.status} failed for ${path}`);
  return response.json();
}

async function main() {
  const sourceRunId = process.argv[2];
  const prepared = JSON.parse(readFileSync(process.argv[3] ?? "prepared-release.json", "utf8"));
  const proof = await validatePreparedDeployment(
    prepared,
    sourceRunId,
    githubRequest,
    process.env.GITHUB_REPOSITORY,
  );
  if (!process.env.GITHUB_OUTPUT) throw new Error("GITHUB_OUTPUT is required");
  appendFileSync(process.env.GITHUB_OUTPUT, `source_sha=${proof.sourceSha}\n`);
  appendFileSync(process.env.GITHUB_OUTPUT, Object.entries(proof.selection)
    .map(([component, selected]) => `${component}=${selected}\n`).join(""));
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : "Prepared release validation failed");
    process.exitCode = 1;
  });
}
