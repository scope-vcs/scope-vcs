import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";

import { validatePreparedDeployment } from "./prepared-deploy-trust.mjs";
import { validateRecoveryPreparation } from "./recovery-preparation-trust.mjs";

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

const replayWorkflow = readFileSync(new URL("../workflows/scope-prepared-deploy.yml", import.meta.url), "utf8");
function replayJob(name) {
  return replayWorkflow.split(`\n  ${name}:\n`)[1].split(/\n  [\w-]+:\n/)[0];
}

function eligible(name, selection, backendResult, cancelled = false) {
  const condition = replayJob(name).match(/\n    if: (.*(?:\n      .*\S)*)/)?.[1]
    .replace(/^>-\n/, "").trim();
  assert.ok(condition, `${name} has an explicit selection condition`);
  const executable = condition
    .replace(/cancelled\(\)/g, JSON.stringify(cancelled))
    .replace(/needs\.prepare\.outputs\.(\w+)/g, (_, component) => JSON.stringify(String(selection[component])))
    .replace(/needs\.prepare\.result/g, '"success"')
    .replace(/needs\.backend-deploy\.result/g, JSON.stringify(backendResult));
  return Boolean(runInNewContext(executable, {}, { timeout: 100 }));
}

for (const [name, selected] of [
  ["web-only", ["web"]],
  ["backend-only", ["api", "worker", "cache", "router", "media", "mediaWorker"]],
  ["full application", ["api", "worker", "cache", "router", "media", "mediaWorker", "web"]],
]) {
  test(`${name} prepared replay selects only manifest components`, async () => {
    const state = fixture();
    state.prepared.components = Object.fromEntries(selected.map((component) => [component, state.prepared.components[component]]));
    if (name === "web-only") delete state.prepared.maintenanceSha256;
    const proof = await validatePreparedDeployment(state.prepared, sourceRunId, state.request, repository);
    const backend = selected.some((component) => component !== "web");
    assert.equal(proof.selection.backend, backend);
    for (const component of ["api", "worker", "cache", "router", "media", "mediaWorker", "web"]) {
      assert.equal(proof.selection[component], selected.includes(component));
      assert.ok(replayWorkflow.includes(`${component}: \${{ steps.source.outputs.${component} }}`));
    }
    assert.equal(eligible("backend-deploy", proof.selection, "skipped"), backend);
    assert.equal(eligible("web-deploy", proof.selection, backend ? "success" : "skipped"), selected.includes("web"));
    assert.equal(eligible("web-deploy", proof.selection, "failure"), false);
    assert.equal(eligible("web-deploy", proof.selection, "cancelled"), false);
    assert.equal(eligible("web-deploy", proof.selection, backend ? "success" : "skipped", true), false);
    state.jobs[2].conclusion = "failure";
    await assert.rejects(validatePreparedDeployment(state.prepared, sourceRunId, state.request, repository), /release-proof/);
  });
}

test("prepared replay rejects empty, unknown, mismatched, and incomplete backend artifacts", async () => {
  for (const [mutate, expected] of [
    [(state) => { state.prepared.components = {}; }, /nonempty/],
    [(state) => { state.prepared.components.unknown = state.prepared.components.web; }, /Unknown release component/],
    [(state) => { state.prepared.components.cli = { ...state.prepared.components.web, serviceId: manifest.services.cli.id }; }, /application components/],
    [(state) => { state.prepared.components.web.serviceId = "wrong"; }, /wrong service/],
    [(state) => { delete state.prepared.components.api; }, /missing api/],
    [(state) => { state.prepared.components.web.sourceSha = mainSha; }, /bind its source/],
  ]) {
    const state = fixture();
    mutate(state);
    await assert.rejects(validatePreparedDeployment(state.prepared, sourceRunId, state.request, repository), expected);
  }
});

test("cutover recovery retains its complete backend requirement", async () => {
  const state = fixture();
  state.prepared.components = { web: state.prepared.components.web };
  await assert.rejects(validateRecoveryPreparation(state.prepared, state.request, repository, manifest), /missing api/);
});

test("replay passes component flags and excludes CLI artifacts outside the prepared manifest", () => {
  for (const [input, component] of Object.entries({ cache: "cache", worker: "worker", media_worker: "mediaWorker", router: "router", media: "media", api: "api" })) {
    assert.ok(replayJob("backend-deploy").includes(`deploy_${input}: \${{ needs.prepare.outputs.${component} == 'true' }}`));
  }
  assert.match(replayJob("web-deploy"), /needs: \[prepare, backend-deploy\]/);
  assert.doesNotMatch(replayWorkflow, /cli-deploy|scope-cli-deploy|artifact_run_id/);
});
