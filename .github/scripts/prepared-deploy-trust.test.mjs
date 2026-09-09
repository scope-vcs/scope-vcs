import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { validatePreparedDeployment } from "./prepared-deploy-trust.mjs";

const repository = "scope-vcs/scope-vcs";
const sourceRunId = "34281642523";
const sourceSha = "a".repeat(40);
const mainSha = "b".repeat(40);
const manifest = JSON.parse(readFileSync(new URL("../deployment-services.json", import.meta.url)));

function fixture() {
  const prepared = {
    schemaVersion: 1,
    sourceSha,
    preparationRunId: sourceRunId,
    maintenanceSha256: "c".repeat(64),
    components: Object.fromEntries(
      ["api", "worker", "cache", "router", "media", "mediaWorker", "web"].map((component) => [
        component,
        {
          image: component === "mediaWorker"
            ? `ghcr.io/scope-vcs/scope-media-worker@sha256:${"d".repeat(64)}`
            : `ghcr.io/${repository}/railway-private-${component}@sha256:${"d".repeat(64)}`,
          serviceId: manifest.services[component].id,
          sourceSha,
        },
      ]),
    ),
  };
  const run = {
    id: Number(sourceRunId),
    event: "workflow_dispatch",
    head_branch: "main",
    head_repository: { id: 1, full_name: repository },
    head_sha: sourceSha,
    path: ".github/workflows/scope-production-deploy.yml",
    repository: { id: 1, full_name: repository },
  };
  const comparison = {
    status: "ahead",
    base_commit: { sha: sourceSha },
    merge_base_commit: { sha: sourceSha },
  };
  const jobs = [
    {
      id: 1,
      run_id: Number(sourceRunId),
      head_sha: sourceSha,
      name: "Production validation gate",
      status: "completed",
      conclusion: "success",
      steps: [],
    },
    {
      id: 2,
      run_id: Number(sourceRunId),
      head_sha: sourceSha,
      name: "Prepare Railway artifacts / prepare",
      status: "completed",
      conclusion: "success",
      steps: [{ name: "Prepare immutable release images", conclusion: "success" }],
    },
  ];
  jobs.push({
    id: 3, run_id: Number(sourceRunId), head_sha: sourceSha,
    name: "Prove prepared release in release-proof", status: "completed", conclusion: "success",
  });
  const request = async (path) => {
    if (path === `/actions/runs/${sourceRunId}`) return structuredClone(run);
    if (path === "/branches/main") return { name: "main", commit: { sha: mainSha } };
    if (path === `/compare/${sourceSha}...${mainSha}`) return structuredClone(comparison);
    if (path === `/actions/runs/${sourceRunId}/jobs?filter=all&per_page=100&page=1`) {
      return { jobs: structuredClone(jobs) };
    }
    throw new Error(`Unexpected request ${path}`);
  };
  return { comparison, jobs, prepared, request, run };
}

test("prepared deploy accepts only its validated main source and exact artifact", async () => {
  const accepted = fixture();
  assert.equal(
    (await validatePreparedDeployment(accepted.prepared, sourceRunId, accepted.request, repository)).sourceSha,
    sourceSha,
  );

  for (const [message, mutate, expected] of [
    ["unvalidated source", (state) => { state.jobs[0].conclusion = "failure"; }, /validation gate/],
    ["unproven release", (state) => { state.jobs[2].conclusion = "skipped"; }, /release-proof/],
    ["non-main source", (state) => { state.run.head_branch = "candidate"; }, /on main/],
    ["non-ancestor source", (state) => { state.comparison.status = "diverged"; }, /main history/],
    ["artifact from another run", (state) => { state.prepared.preparationRunId = "123"; }, /source run ID/],
  ]) {
    const state = fixture();
    mutate(state);
    await assert.rejects(
      validatePreparedDeployment(state.prepared, sourceRunId, state.request, repository),
      expected,
      message,
    );
  }
});
