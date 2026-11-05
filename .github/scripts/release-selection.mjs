#!/usr/bin/env node

import { appendFileSync, readFileSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { findOpenCutover } from "./release-cutover-journal.mjs";
import { githubRequest, latestSuccessfulDeployments, successfulReleaseHistory } from "./production-deployment-progress.mjs";
import { pathToFileURL } from "node:url";

import { validatePreparedRelease } from "./railway-artifact.mjs";
import { validateRecoveryPreparation } from "./recovery-preparation-trust.mjs";
import { APPLICATION_COMPONENTS, BACKEND_COMPONENTS, backendSelected, deploymentComponent } from "./deployment-components.mjs";

const components = APPLICATION_COMPONENTS;
const validationJobName = "Validate selected components / Production validation gate";

const smokeInput = (filename) => filename === "dev/media-smoke.mjs"
  || filename.startsWith("web/smoke/")
  || filename === "web/scripts/update-smoke-server-functions.mjs"
  || /^\.github\/scripts\/[^/]*smoke/.test(filename);

export function assertReusableStagingSource(comparison, { allowSmokeChanges = false } = {}) {
  if (!Array.isArray(comparison?.files) || comparison.files.length >= 300) {
    throw new Error("Cannot establish unchanged staging inputs for source reuse");
  }
  const stagingInput = (filename) => typeof filename === "string" && !(allowSmokeChanges && smokeInput(filename)) && (
    filename === ".github/deployment-services.json"
    || filename === "dev/tool-versions.json"
    || filename === "dev/check-git-version.mjs"
    || filename === "dev/media-smoke.mjs"
    || /^\.github\/workflows\/(release|validate|validate-server|prepare-release|prepare-smoke-tools|deploy-staging|deploy-backend|deploy-web)\.yml$/.test(filename)
    || /^\.github\/scripts\/(staging|railway|prepare-railway|extract-railway|extract-staging-web-manifest|deploy-|stop-railway|verify-staging|[^/]*smoke|recovery-preparation-trust|release-selection)/.test(filename)
    || filename.startsWith("crates/scope-postgres/src/migrations/")
    || filename.startsWith("deploy/postgres/")
    || filename.startsWith("deploy/railway/")
    || filename.startsWith("web/smoke/")
    || filename.startsWith("web/scripts/update-smoke-server-functions.mjs")
    || /(?:^|\/)(?:tests?\/|[^/]+\.(?:test|spec)\.)/.test(filename)
  );
  const relevant = comparison.files.find(({ filename, previous_filename: previousFilename }) =>
    stagingInput(filename) || stagingInput(previousFilename));
  if (relevant) throw new Error(`Staging evidence is stale after ${relevant.filename} changed`);
}

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

  const deployment = deploymentManifest();
  validatePreparedRelease(prepared, {
    services: deployment.services,
    sourceSha: prepared.sourceSha,
  });
  const selected = Object.keys(prepared.components);
  if (selected.length === 0 || selected.some((component) => !components.includes(component))) {
    throw new Error("Prepared deployment requires a nonempty set of application components");
  }
  const backend = backendSelected(Object.fromEntries(selected.map(component => [component, true])));
  if (backend) validatePreparedRelease(prepared, { components: BACKEND_COMPONENTS });

  const responses = new Map();
  const cachedRequest = (path) => {
    if (!responses.has(path)) responses.set(path, Promise.resolve(request(path)));
    return responses.get(path);
  };
  const proof = await validateRecoveryPreparation(prepared, cachedRequest, repository, deployment, selected);
  if (resumeStaging) {
    const run = await cachedRequest(`/actions/runs/${sourceRunId}`);
    if (run.status !== 'completed') throw new Error('Staging resume requires a completed source run');
  }

  let validated = false;
  let staging;
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
        if (job.name === 'Deploy staging / Deploy and smoke staging'
            && (!staging || job.run_attempt > staging.run_attempt)) staging = job;
      }
    }
    if (result.jobs.length < 100) break;
  }
  const stagingPassed = staging?.conclusion === 'success';
  const staged = stagingPassed || (resumeStaging
    && ['failure', 'cancelled'].includes(staging?.conclusion)
    && Boolean(staging.steps?.some(step => step.name === 'Deploy candidate once' && step.conclusion === 'success')));
  if (!validated || !staged) {
    throw new Error("Source run did not pass the production validation gate and staging deployment");
  }
  if (proof.mainSha !== proof.sourceSha) {
    assertReusableStagingSource(await cachedRequest(`/compare/${proof.sourceSha}...${proof.mainSha}`),
      { allowSmokeChanges: resumeStaging && !stagingPassed });
  }
  if (resumeStaging && !stagingPassed) validatePreparedRelease(prepared, { components });
  return { ...proof, stagingPassed };
}

function selection(prepared, recoveryId = "", resumeStaging = false, selected = Object.keys(prepared.components)) {
  const flags = Object.fromEntries(components.map(component => [component, selected.includes(component)]));
  flags.backend = backendSelected(flags);
  return { sha: prepared.sourceSha, recover_cutover_id: recoveryId,
    recover_components: flags, prepared_run_id: prepared.preparationRunId,
    resume_staging: resumeStaging, prepared };
}

async function selectRollback({ rollbackTo = "", repository, releases, deployments, loadPrepared,
  configChanged }, request) {
  if (rollbackTo && !/^[0-9a-f]{40}$/.test(rollbackTo)) throw new Error("rollback_to must be a full commit SHA");
  const [current, ...earlier] = releases;
  if (!current) throw new Error("Rollback requires a verified production release");
  const replaced = new Set([current.receipt.sourceSha, current.receipt.rollbackOf].filter(Boolean));
  const newer = new Set([current.receipt.sourceSha]);
  const target = earlier.find(({ receipt }) => {
    const found = rollbackTo ? receipt.sourceSha === rollbackTo : !replaced.has(receipt.sourceSha);
    if (!found) {
      newer.add(receipt.sourceSha);
      if (receipt.rollbackOf) replaced.add(receipt.rollbackOf);
    }
    return found;
  });
  if (!target) {
    throw new Error(rollbackTo ? `${rollbackTo} has no earlier verified production release` : "No earlier verified production release exists");
  }
  const targetSha = target.receipt.sourceSha;
  if (!target.runId) throw new Error(`The ${targetSha} release receipt does not name its Release run`);
  const prepared = await loadPrepared(target.runId, targetSha);
  validatePreparedRelease(prepared, { sourceSha: targetSha, services: deploymentManifest().services });
  await validateRecoveryPreparation(prepared, request, repository);

  const selected = components.filter(component => {
    const deployed = deployments[component]?.sourceSha;
    return deployed !== targetSha && newer.has(deployed);
  });
  if (selected.length === 0) throw new Error(`Production already runs ${targetSha}`);
  const missing = selected.filter(component => !prepared.components[component]);
  if (missing.length) {
    throw new Error(`The ${targetSha} release did not deploy ${missing.join(", ")}; choose an older rollback_to`);
  }
  const runtimeConfigs = selected.map(component => deploymentComponent(component).runtimeConfig);
  if (configChanged(targetSha, runtimeConfigs)) {
    throw new Error(`Railway runtime config changed after ${targetSha}; roll forward instead`);
  }
  return { ...selection(prepared, "", false, selected), rollback: true, rollback_of: current.receipt.sourceSha };
}

export async function selectRelease({ sourceSha, sourceRunId = "", resumeStaging = false, rollback = false,
  rollbackTo = "", scheduled = false, repository, loadPrepared, loadReleases, loadDeployments, configChanged }, request) {
  if (resumeStaging && !sourceRunId) throw new Error('Staging resume requires its source run ID');
  if (rollbackTo && !rollback) throw new Error('rollback_to requires rollback');
  if (rollback && sourceRunId) throw new Error('Rollback cannot reuse source_run_id');
  const journal = await findOpenCutover(request);
  if (journal) {
    if (rollback) throw new Error('Recover the unresolved production cutover before rolling back');
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
    const proof = await validatePreparedDeployment(prepared, sourceRunId, request, repository, { resumeStaging });
    return selection(prepared, '', resumeStaging && !proof.stagingPassed);
  }
  if (rollback) {
    return selectRollback({ rollbackTo, repository, releases: await loadReleases(), deployments: await loadDeployments(),
      loadPrepared, configChanged }, request);
  }
  if (!/^[0-9a-f]{40}$/.test(sourceSha ?? "")) throw new Error("Source SHA must be a full commit SHA");
  const release = { sha: sourceSha, recover_cutover_id: "", recover_components: {}, prepared_run_id: "", resume_staging: false };
  if (scheduled) {
    const [latest] = await loadReleases();
    if (latest?.receipt.rollbackOf) return { ...release, paused_after_rollback_of: latest.receipt.rollbackOf };
  }
  return release;
}

function deploymentManifest() {
  return JSON.parse(readFileSync(new URL("../deployment-services.json", import.meta.url), "utf8"));
}

function runtimeConfigChanged(sha, paths) {
  try {
    execFileSync("git", ["diff", "--quiet", sha, "HEAD", "--", ...paths], { stdio: "pipe" });
    return false;
  } catch (error) {
    if (error.status === 1) return true;
    throw error;
  }
}

async function downloadPrepared(runId, sourceSha) {
  const artifactSha = sourceSha ?? (await githubRequest(`/actions/runs/${runId}`)).head_sha;
  if (!/^[0-9a-f]{40}$/.test(artifactSha ?? "")) throw new Error("Source run has no valid commit SHA");
  const directory = mkdtempSync(join(tmpdir(), "scope-prepared-"));
  try {
    execFileSync("gh", ["run", "download", runId, "--repo", process.env.GITHUB_REPOSITORY,
      "--name", `prepared-release-${artifactSha}`, "--dir", directory], { stdio: "pipe" });
    return JSON.parse(readFileSync(join(directory, "prepared-release.json"), "utf8"));
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

async function main() {
  const result = await selectRelease({ sourceSha: process.env.SOURCE_SHA || process.env.GITHUB_SHA,
    sourceRunId: process.env.SOURCE_RUN_ID || "", resumeStaging: process.env.RESUME_STAGING === 'true',
    rollback: process.env.ROLLBACK === 'true', rollbackTo: process.env.ROLLBACK_TO || "",
    scheduled: Boolean(process.env.SCHEDULE_INTENT), repository: process.env.GITHUB_REPOSITORY,
    loadPrepared: downloadPrepared, loadReleases: () => successfulReleaseHistory(20),
    loadDeployments: () => latestSuccessfulDeployments(), configChanged: runtimeConfigChanged }, githubRequest);
  const { prepared, ...outputs } = result;
  if (prepared) writeFileSync("selected-release.json", `${JSON.stringify(prepared, null, 2)}\n`);
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT,
    Object.entries(outputs).map(([key, value]) => `${key}=${typeof value === "string" ? value : JSON.stringify(value)}\n`).join(""));
  else process.stdout.write(`${JSON.stringify(outputs)}\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
