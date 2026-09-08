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
    components,
    services: manifest.services,
    sourceSha: prepared.sourceSha,
  });

  const responses = new Map();
  const cachedRequest = (path) => {
    if (!responses.has(path)) responses.set(path, Promise.resolve(request(path)));
    return responses.get(path);
  };
  const proof = await validateRecoveryPreparation(prepared, cachedRequest, repository, manifest);

  for (let page = 1; ; page += 1) {
    const result = await cachedRequest(
      `/actions/runs/${sourceRunId}/jobs?filter=all&per_page=100&page=${page}`,
    );
    if (!Array.isArray(result.jobs)) throw new Error("Cannot read source validation jobs");
    const validated = result.jobs.some((job) => (
      job.name === validationJobName
      && String(job.run_id) === sourceRunId
      && job.head_sha === proof.sourceSha
      && job.status === "completed"
      && job.conclusion === "success"
    ));
    if (validated) return proof;
    if (result.jobs.length < 100) break;
  }
  throw new Error("Source run did not pass the production validation gate");
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
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(error instanceof Error ? error.message : "Prepared release validation failed");
    process.exitCode = 1;
  });
}
