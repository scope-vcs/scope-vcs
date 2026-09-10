#!/usr/bin/env node

import { appendFileSync, readFileSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { findOpenCutover } from "./release-cutover-journal.mjs";
import { githubRequest } from "./production-deployment-progress.mjs";
import { pathToFileURL } from "node:url";

import { validatePreparedRelease } from "./railway-artifact.mjs";
import { validateRecoveryPreparation } from "./recovery-preparation-trust.mjs";

const components = ["api", "run-worker", "cache", "git-router", "media-api", "media-worker", "web"];
const validationJobName = "Validate selected components / Production validation gate";

export async function validatePreparedDeployment(
  prepared,
  sourceRunId,
  request,
  repository,
  { resumeStaging = false } = {},
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
  if (resumeStaging) {
    validatePreparedRelease(prepared, { components });
    const run = await cachedRequest(`/actions/runs/${sourceRunId}`);
    if (run.status !== 'completed') throw new Error('Staging resume requires a completed source run');
  }
  const selection = { backend, ...Object.fromEntries(components.map((component) => [component, selected.includes(component)])) };

  let validated = false;
  let staged = false;
  for (let page = 1; ; page += 1) {
    const result = await cachedRequest(
      `/actions/runs/${sourceRunId}/jobs?filter=all&per_page=100&page=${page}`,
    );
    if (!Array.isArray(result.jobs)) throw new Error("Cannot read source validation jobs");
    for (const job of result.jobs) {
      if (String(job.run_id) === sourceRunId
          && job.head_sha === proof.sourceSha
          && job.status === "completed") {
        if (job.name === validationJobName && job.conclusion === 'success') validated = true;
        if (job.name === 'Deploy staging / Deploy and smoke staging') {
          if (resumeStaging) {
            staged ||= ['success', 'failure', 'cancelled'].includes(job.conclusion)
              && job.steps?.some(step => step.name === 'Deploy candidate once' && step.conclusion === 'success');
          } else staged ||= job.conclusion === 'success';
        }
      }
    }
    if (validated && staged) return { ...proof, selection };
    if (result.jobs.length < 100) break;
  }
  throw new Error("Source run did not pass the production validation gate and staging deployment");
}


function selection(prepared, recoveryId = "", resumeStaging = false) {
  const flags = Object.fromEntries(components.map(component => [component, Boolean(prepared.components[component])]));
  flags.backend = components.some(component => component !== "web" && flags[component]);
  return { sha: prepared.sourceSha, recover_cutover_id: recoveryId,
    recover_components: flags, reuse_components: flags, prepared_run_id: prepared.preparationRunId,
    resume_staging: resumeStaging, prepared };
}

export async function selectRelease({ sourceSha, sourceRunId = "", recoveryId = "", resumeStaging = false, repository,
  loadPrepared }, request) {
  if (resumeStaging && !sourceRunId) throw new Error('Staging resume requires its source run ID');
  const journal = await findOpenCutover(request);
  if (recoveryId && journal?.id !== recoveryId) throw new Error("Requested recovery is not the sole unresolved cutover");
  if (journal) {
    if (resumeStaging) throw new Error('Recover the unresolved production cutover before resuming staging');
    if (sourceRunId && sourceRunId !== journal.prepared.preparationRunId) {
      throw new Error("An unresolved cutover must recover its original preparation run");
    }
    await validateRecoveryPreparation(journal.prepared, request, repository);
    return selection(journal.prepared, journal.id);
  }
  if (sourceRunId) {
    if (!/^[1-9][0-9]*$/.test(sourceRunId)) throw new Error("Source run ID must be numeric");
    const prepared = await loadPrepared(sourceRunId);
    await validatePreparedDeployment(prepared, sourceRunId, request, repository, { resumeStaging });
    return selection(prepared, '', resumeStaging);
  }
  if (!/^[0-9a-f]{40}$/.test(sourceSha ?? "")) throw new Error("Source SHA must be a full commit SHA");
  return { sha: sourceSha, recover_cutover_id: "", recover_components: {}, reuse_components: {}, prepared_run_id: "", resume_staging: false };
}

async function downloadPrepared(runId) {
  const run = await githubRequest(`/actions/runs/${runId}`);
  if (!/^[0-9a-f]{40}$/.test(run.head_sha ?? "")) throw new Error("Source run has no valid commit SHA");
  const directory = mkdtempSync(join(tmpdir(), "scope-prepared-"));
  try {
    execFileSync("gh", ["run", "download", runId, "--repo", process.env.GITHUB_REPOSITORY,
      "--name", `prepared-release-${run.head_sha}`, "--dir", directory], { stdio: "pipe" });
    return JSON.parse(readFileSync(join(directory, "prepared-release.json"), "utf8"));
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

async function main() {
  const result = await selectRelease({ sourceSha: process.env.SOURCE_SHA || process.env.GITHUB_SHA,
    sourceRunId: process.env.SOURCE_RUN_ID || "", recoveryId: process.env.RECOVER_CUTOVER_ID || "",
    resumeStaging: process.env.RESUME_STAGING === 'true',
    repository: process.env.GITHUB_REPOSITORY, loadPrepared: downloadPrepared }, githubRequest);
  const { prepared, ...outputs } = result;
  if (prepared) writeFileSync("selected-release.json", `${JSON.stringify(prepared, null, 2)}\n`);
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT,
    Object.entries(outputs).map(([key, value]) => `${key}=${typeof value === "string" ? value : JSON.stringify(value)}\n`).join(""));
  else process.stdout.write(`${JSON.stringify(outputs)}\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
