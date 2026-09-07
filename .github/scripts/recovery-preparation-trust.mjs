import { validatePreparedRelease } from "./railway-artifact.mjs";

const workflowPath = ".github/workflows/scope-production-deploy.yml";
const preparationJobName = "Prepare Railway artifacts / prepare";
const preparationStepName = "Prepare immutable release images";
const shaPattern = /^[0-9a-f]{40}$/;

// The production workflow and authorized operators own journal writes and these GHCR
// packages. This gate rejects PR/candidate preparation; it is not a signature over a
// journal written by an actor who already has those production publishing privileges.
export async function validateRecoveryPreparation(prepared, request, repository) {
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository ?? "")) {
    throw new Error("Recovery requires the trusted GITHUB_REPOSITORY");
  }
  validatePreparedRelease(prepared, { components: ["api", "worker", "cache", "router"] });
  const { sourceSha, preparationRunId } = prepared;
  if (!/^[1-9][0-9]*$/.test(preparationRunId ?? "")) {
    throw new Error("Recovery requires its original preparation run ID");
  }
  const owner = repository.toLowerCase();
  for (const [component, artifact] of Object.entries(prepared.components)) {
    if (artifact.image.split("@")[0] !== `ghcr.io/${owner}/railway-${component}`) {
      throw new Error(`Recovery ${component} image is outside its trusted production package`);
    }
  }

  const run = await request(`/actions/runs/${preparationRunId}`);
  if (String(run.id) !== preparationRunId
      || run.repository?.full_name?.toLowerCase() !== owner
      || run.head_repository?.full_name?.toLowerCase() !== owner
      || !Number.isSafeInteger(run.repository?.id)
      || run.repository.id <= 0
      || run.head_repository?.id !== run.repository.id
      || run.path !== workflowPath
      || run.head_branch !== "main"
      || run.head_sha !== sourceSha
      || !["push", "workflow_dispatch"].includes(run.event)) {
    throw new Error("Recovery preparation must come from this repository's production workflow on main at the exact source SHA");
  }

  // Resolve the branch before comparing immutable SHAs so a tag named main cannot
  // satisfy ancestry. No dependency on Actions artifact/log retention is needed.
  const main = await request("/branches/main");
  const mainSha = main.commit?.sha;
  if (main.name !== "main" || !shaPattern.test(mainSha ?? "")) {
    throw new Error("Cannot establish trusted main history for recovery");
  }
  const comparison = await request(`/compare/${sourceSha}...${mainSha}`);
  if (!["ahead", "identical"].includes(comparison.status)
      || comparison.base_commit?.sha !== sourceSha
      || comparison.merge_base_commit?.sha !== sourceSha) {
    throw new Error("Recovery source SHA is not in trusted main history");
  }

  // A cutover can kill or fail the overall run after preparation completed. A rerun
  // of failed jobs can also omit preparation, so retain successful earlier attempts.
  for (let page = 1; ; page += 1) {
    const result = await request(`/actions/runs/${preparationRunId}/jobs?filter=all&per_page=100&page=${page}`);
    if (!Array.isArray(result.jobs)) throw new Error("Cannot read original preparation jobs");
    const job = result.jobs.find((candidate) => (
      candidate.name === preparationJobName
      && String(candidate.run_id) === preparationRunId
      && candidate.head_sha === sourceSha
      && candidate.status === "completed"
      && candidate.conclusion === "success"
      && candidate.steps?.some((step) => (
        step.name === preparationStepName && step.conclusion === "success"
      ))
    ));
    if (job) return { sourceSha, preparationRunId, preparationJobId: job.id, mainSha };
    if (result.jobs.length < 100) break;
  }
  throw new Error("The original production preparation job did not successfully build the release images");
}
