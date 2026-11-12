import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { validatePreparedDeployment, selectRelease } from "./release-selection.mjs";
import { validateRecoveryPreparation } from "./recovery-preparation-trust.mjs";

const repository = "scope-vcs/scope-vcs";
const sourceRunId = "34281642523";
const sourceSha = "a".repeat(40);
const mainSha = "b".repeat(40);
const manifest = JSON.parse(readFileSync(new URL("../../.github/deployment-services.json", import.meta.url)));

function fixture() {
  const prepared = {
    schemaVersion: 1,
    sourceSha,
    preparationRunId: sourceRunId,
    maintenanceSha256: "c".repeat(64),
    components: Object.fromEntries(
      ["api", "run-worker", "git-router", "media-api", "media-worker", "web"].map((component) => [
        component,
        {
          image: component === "media-worker"
            ? `ghcr.io/scope-vcs/scope-media-worker@sha256:${"d".repeat(64)}`
            : `ghcr.io/${repository}/railway-private-${({"run-worker":"worker","git-router":"router","media-api":"media"})[component] ?? component}@sha256:${"d".repeat(64)}`,
          serviceId: manifest.services[component].id,
          sourceSha,
        },
      ]),
    ),
  };
  const run = {
    id: Number(sourceRunId),
    status: 'completed',
    event: "workflow_dispatch",
    head_branch: "main",
    head_repository: { id: 1, full_name: repository },
    head_sha: sourceSha,
    path: ".github/workflows/release.yml",
    repository: { id: 1, full_name: repository },
  };
  const comparison = {
    status: "ahead",
    base_commit: { sha: sourceSha },
    merge_base_commit: { sha: sourceSha },
    files: [],
  };
  const jobs = [
    {
      id: 1,
      run_id: Number(sourceRunId),
      head_sha: sourceSha,
      name: "Validate selected components / Production validation gate",
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
    id: 3, run_id: Number(sourceRunId), run_attempt: 1, head_sha: sourceSha,
    name: "Deploy staging / Deploy and smoke staging", status: "completed", conclusion: "success",
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
    ["unproven release", (state) => { state.jobs[2].conclusion = "skipped"; }, /staging/],
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

test('staging resume accepts a failed smoke run only after successful validation and complete deployment', async () => {
  const state = fixture();
  state.jobs[2].conclusion = 'failure';
  state.jobs[2].steps = [{ name: 'Deploy candidate once', conclusion: 'success' }];
  const result = await selectRelease({ sourceSha: mainSha, sourceRunId, resumeStaging: true, repository,
    loadPrepared: async () => state.prepared }, path => path.startsWith('/deployments?') ? [] : state.request(path));
  assert.equal(result.sha, sourceSha);
  assert.equal(result.prepared_run_id, sourceRunId);
  assert.equal(result.resume_staging, true);
  assert.deepEqual(result.prepared, state.prepared);
  await assert.rejects(validatePreparedDeployment(state.prepared, sourceRunId, state.request, repository), /staging/);
});

test('a requested resume reuses staging that already passed', async () => {
  const state = fixture();
  const result = await selectRelease({ sourceSha: mainSha, sourceRunId, resumeStaging: true, repository,
    loadPrepared: async () => state.prepared }, path => path.startsWith('/deployments?') ? [] : state.request(path));
  assert.equal(result.resume_staging, false);
  assert.equal(result.prepared_run_id, sourceRunId);

  delete state.prepared.components.web;
  const backendOnly = await selectRelease({ sourceSha: mainSha, sourceRunId, resumeStaging: true, repository,
    loadPrepared: async () => state.prepared }, path => path.startsWith('/deployments?') ? [] : state.request(path));
  assert.equal(backendOnly.resume_staging, false);
});

test('a failed staging rerun overrides an earlier successful attempt', async () => {
  const state = fixture();
  state.jobs.push({
    id: 4, run_id: Number(sourceRunId), run_attempt: 2, head_sha: sourceSha,
    name: 'Deploy staging / Deploy and smoke staging', status: 'completed', conclusion: 'failure',
    steps: [{ name: 'Deploy candidate once', conclusion: 'success' }],
  });
  await assert.rejects(validatePreparedDeployment(state.prepared, sourceRunId, state.request, repository), /staging/);
  const result = await selectRelease({ sourceSha: mainSha, sourceRunId, resumeStaging: true, repository,
    loadPrepared: async () => state.prepared }, path => path.startsWith('/deployments?') ? [] : state.request(path));
  assert.equal(result.resume_staging, true);
});

test('a smoke resume permits corrected smoke inputs but not other staging inputs', async () => {
  const select = (state) => selectRelease({ sourceSha: mainSha, sourceRunId, resumeStaging: true, repository,
    loadPrepared: async () => state.prepared }, path => path.startsWith('/deployments?') ? [] : state.request(path));
  for (const filename of ['web/smoke/resource-navigation.spec.mjs', 'dev/media-smoke.mjs', 'deploy/scripts/staging-git-smoke.sh']) {
    const state = fixture();
    state.jobs[2].conclusion = 'failure';
    state.jobs[2].steps = [{ name: 'Deploy candidate once', conclusion: 'success' }];
    state.comparison.files = [{ filename }];
    assert.equal((await select(state)).resume_staging, true, filename);
    const passed = fixture();
    passed.comparison.files = [{ filename }];
    await assert.rejects(select(passed), /Staging evidence is stale/, `${filename} after passed staging`);
  }
  const state = fixture();
  state.jobs[2].conclusion = 'failure';
  state.jobs[2].steps = [{ name: 'Deploy candidate once', conclusion: 'success' }];
  state.comparison.files = [{ filename: 'crates/scope-postgres/src/migrations/m0062_new_table.rs' }];
  await assert.rejects(select(state), /Staging evidence is stale/);
});

test('source reuse rejects changed staging inputs but permits receipt-only repair', async () => {
  for (const filename of [
    '.github/deployment-services.json',
    'dev/tool-versions.json',
    'dev/check-git-version.mjs',
    'dev/media-smoke.mjs',
    'deploy/scripts/staging-resume.mjs',
    'deploy/scripts/stop-railway-writers.sh',
    'deploy/scripts/verify-staging-target.mjs',
    'deploy/scripts/deploy-railway-image.mjs',
    '.github/workflows/prepare-smoke-tools.yml',
    'web/smoke/server-functions-smoke.mjs',
    'crates/scope-postgres/src/migrations/m0062_new_table.rs',
    'apps/web/src/history.test.ts',
  ]) {
    const state = fixture();
    state.comparison.files = [{ filename }];
    await assert.rejects(selectRelease({ sourceSha: mainSha, sourceRunId, resumeStaging: true, repository,
      loadPrepared: async () => state.prepared }, path => path.startsWith('/deployments?') ? [] : state.request(path)),
    /Staging evidence is stale/);
  }
  const state = fixture();
  state.comparison.files = [{ filename: 'deploy/scripts/production-deployment-progress.mjs' }];
  const result = await selectRelease({ sourceSha: mainSha, sourceRunId, resumeStaging: true, repository,
    loadPrepared: async () => state.prepared }, path => path.startsWith('/deployments?') ? [] : state.request(path));
  assert.equal(result.resume_staging, false);
});

test('staging resume rejects incomplete deployment, unvalidated images, and unrelated attempts', async () => {
  for (const mutate of [
    state => { state.jobs[2].steps[0].conclusion = 'failure'; },
    state => { state.jobs[2].steps = []; },
    state => { state.jobs[2].conclusion = 'skipped'; },
    state => { state.jobs[2].head_sha = mainSha; },
    state => { state.jobs[2].run_id = 999; },
    state => { state.jobs[0].conclusion = 'failure'; },
    state => { state.run.status = 'in_progress'; },
    state => { delete state.prepared.components.web; },
  ]) {
    const state = fixture();
    state.jobs[2].conclusion = 'failure';
    state.jobs[2].steps = [{ name: 'Deploy candidate once', conclusion: 'success' }];
    mutate(state);
    await assert.rejects(validatePreparedDeployment(state.prepared, sourceRunId, state.request, repository,
      { resumeStaging: true }));
  }
  await assert.rejects(selectRelease({ sourceSha, resumeStaging: true, repository }, async () => []), /source run ID/);
});

for (const [name, selected] of [
  ["web-only", ["web"]],
  ["backend-only", ["api", "run-worker", "git-router", "media-api", "media-worker"]],
  ["full application", ["api", "run-worker", "git-router", "media-api", "media-worker", "web"]],
]) {
  test(`${name} prepared replay selects only manifest components`, async () => {
    const state = fixture();
    state.prepared.components = Object.fromEntries(selected.map((component) => [component, state.prepared.components[component]]));
    if (name === "web-only") delete state.prepared.maintenanceSha256;
    assert.equal((await validatePreparedDeployment(state.prepared, sourceRunId, state.request, repository)).sourceSha, sourceSha);
    state.jobs[2].conclusion = "failure";
    await assert.rejects(validatePreparedDeployment(state.prepared, sourceRunId, state.request, repository), /staging/);
  });
}

test("prepared replay rejects empty, unknown, mismatched, and incomplete backend artifacts", async () => {
  for (const [mutate, expected] of [
    [(state) => { state.prepared.components = {}; }, /nonempty/],
    [(state) => { state.prepared.components.unknown = state.prepared.components.web; }, /Unknown release component/],
    [(state) => { state.prepared.components["cli-downloads"] = { ...state.prepared.components.web, serviceId: manifest.services["cli-downloads"].id }; }, /application components/],
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
  delete state.prepared.components.api;
  await assert.rejects(validateRecoveryPreparation(state.prepared, state.request, repository, manifest), /missing api/);
});

test("ordinary selection pins requested revision without downloading artifacts", async () => {
  const result = await selectRelease({ sourceSha, repository,
    loadPrepared: () => { throw new Error("unexpected download"); } }, async () => []);
  assert.equal(result.sha, sourceSha);
  assert.equal(result.prepared_run_id, "");
});

test("replay selects only validated manifest components", async () => {
  const state = fixture();
  const result = await selectRelease({ sourceSha: mainSha, sourceRunId, repository,
    loadPrepared: async () => state.prepared }, path => path.startsWith("/deployments?") ? [] : state.request(path));
  assert.equal(result.sha, sourceSha);
  assert.equal(result.prepared_run_id, sourceRunId);
  assert.equal(result.recover_cutover_id, "");
  assert.equal(result.recover_components.web, true);
  assert.deepEqual(result.prepared, state.prepared);
});

test("recovery overrides new main and rejects a different replay run", async () => {
  const state = fixture();
  const request = path => {
    if (path.startsWith("/deployments?")) return [{ id: 77 }];
    if (path === "/deployments/77") return { id: 77, environment: "production/maintenance", sha: sourceSha,
      payload: { kind: "scope-release-cutover", prepared: state.prepared, baseline: {pending: []}, previous: {} } };
    if (path.startsWith("/deployments/77/statuses")) return [{ description: "cutover:committed" }];
    return state.request(path);
  };
  const result = await selectRelease({ sourceSha: mainSha, repository }, request);
  assert.equal(result.sha, sourceSha);
  assert.equal(result.recover_cutover_id, "77");
  assert.deepEqual(result.prepared, state.prepared);
  await assert.rejects(selectRelease({ sourceSha: mainSha, sourceRunId: "999", repository }, request), /original preparation/);
  await assert.rejects(selectRelease({ sourceSha: mainSha, sourceRunId, resumeStaging: true, repository }, request), /production cutover/);
  await assert.rejects(selectRelease({ sourceSha: mainSha, rollback: true, repository }, request), /before rolling back/);

  for (const [mutate, expected] of [
    [() => { state.run.head_branch = "candidate"; }, /on main/],
    [() => { state.comparison.status = "diverged"; }, /main history/],
    [() => { state.jobs[1].conclusion = "failure"; }, /preparation job/],
  ]) {
    const original = structuredClone({ run: state.run, comparison: state.comparison, jobs: state.jobs });
    mutate();
    await assert.rejects(selectRelease({ sourceSha: mainSha, repository }, request), expected);
    Object.assign(state.run, original.run);
    Object.assign(state.comparison, original.comparison);
    state.jobs.splice(0, state.jobs.length, ...original.jobs);
  }
});

const badSha = "e".repeat(40);
const applicationComponents = ["api", "run-worker", "git-router", "media-api", "media-worker", "web"];
const backendComponents = applicationComponents.filter(component => component !== "web");

function release(sha, evidence, runId, deployed = applicationComponents, rollbackOf) {
  return { receipt: { sourceSha: sha, components: Object.fromEntries(deployed
    .map(component => [component, { sourceSha: sha, evidenceId: `${evidence}-${component}` }])),
  ...(rollbackOf ? { rollbackOf } : {}) }, runId };
}

function live(evidence, deployed = applicationComponents) {
  return Object.fromEntries(deployed.map(component => [component, { evidenceId: `${evidence}-${component}` }]));
}

function rollback(state, { releases, deployments, rollbackTo = "", configChanged = () => false } = {}) {
  state.downloads = [];
  return selectRelease({ sourceSha: mainSha, rollback: true, rollbackTo, repository,
    loadReleases: async () => releases ?? [release(badSha, "bad", "901"), release(sourceSha, "good", "555")],
    loadDeployments: async () => deployments ?? live("bad"),
    loadPrepared: async (...args) => { state.downloads.push(args); return state.preparedByRun?.[args[0]] ?? state.prepared; },
    configChanged }, path => {
    if (path.startsWith("/deployments?")) return [];
    const releaseRun = path.match(/^\/actions\/runs\/([0-9]+)$/)?.[1];
    if (releaseRun && releaseRun !== sourceRunId) {
      return { ...state.run, id: Number(releaseRun), head_sha: mainSha, conclusion: "success", ...state.releaseRun };
    }
    return state.request(path);
  });
}

function rolledBack(result) {
  return applicationComponents.filter(component => result.recover_components[component]);
}

test("rollback redeploys the previous verified release without staging", async () => {
  const state = fixture();
  const result = await rollback(state);
  assert.deepEqual(state.downloads, [["555", sourceSha]]);
  assert.equal(result.sha, sourceSha);
  assert.equal(result.prepared_run_id, sourceRunId);
  assert.equal(result.resume_staging, false);
  assert.equal(result.rollback, true);
  assert.equal(result.rollback_of, badSha);
  assert.deepEqual(rolledBack(result), applicationComponents);
});

test("rollback restores only components a later release changed, including from a web-only target", async () => {
  const backendSha = "8".repeat(40);
  const state = fixture();
  state.prepared.components = { web: state.prepared.components.web };
  delete state.prepared.maintenanceSha256;
  const result = await rollback(state, {
    releases: [release(badSha, "bad", "901", ["web"]), release(sourceSha, "good", "555", ["web"]),
      release(backendSha, "backend", "400", backendComponents)],
    deployments: { ...live("backend", backendComponents), ...live("bad", ["web"]) },
  });
  assert.deepEqual(rolledBack(result), ["web"]);
  assert.equal(result.recover_components.backend, false);
});

test("repeated rollbacks skip every release they rolled back from", async () => {
  const rolledBackSha = "f".repeat(40);
  const rolledBackAgain = "9".repeat(40);
  const state = fixture();
  const result = await rollback(state, {
    releases: [
      release(rolledBackAgain, "again-rollback", "905", applicationComponents, rolledBackSha),
      release(rolledBackSha, "first-rollback", "903", applicationComponents, badSha),
      release(badSha, "bad", "901"),
      release(rolledBackSha, "rolled-back", "700"),
      release(rolledBackAgain, "again", "650"),
      release(sourceSha, "good", "555"),
    ],
    deployments: live("again-rollback"),
  });
  assert.deepEqual(state.downloads, [["555", sourceSha]]);
  assert.equal(result.rollback_of, rolledBackAgain);
  assert.deepEqual(rolledBack(result), applicationComponents);
});

test("a partial rollback receipt does not hide the full release of the same commit", async () => {
  const webOnlySha = "7".repeat(40);
  const state = fixture();
  const result = await rollback(state, {
    releases: [release(badSha, "bad", "901", backendComponents),
      release(sourceSha, "web-rollback", "555", ["web"], webOnlySha),
      release(webOnlySha, "web-only", "800", ["web"]),
      release(sourceSha, "good", "700")],
    deployments: { ...live("bad", backendComponents), ...live("web-rollback", ["web"]) },
  });
  assert.deepEqual(rolledBack(result), backendComponents);
  assert.deepEqual(state.downloads, [["700", sourceSha]]);
});

test("rollback refuses images that differ from the run whose receipt proved them", async () => {
  const state = fixture();
  const rebuild = structuredClone(state.prepared);
  rebuild.components.api.image = rebuild.components.api.image.replace(/d{64}$/, "0".repeat(64));
  state.preparedByRun = { 555: rebuild };
  await assert.rejects(rollback(state, {
    releases: [release(badSha, "bad", "901"), release(sourceSha, "api-rebuild", "555", ["api"]),
      release(sourceSha, "good", "700")],
    deployments: live("bad"),
  }), /differently built api images/);
});

test("rollback refuses targets it cannot restore exactly", async () => {
  for (const [name, mutate, options, expected] of [
    ["target deployed only some changed components", () => {},
      { releases: [release(badSha, "bad", "901"), release(sourceSha, "good", "555", ["api"])] }, /did not deploy run-worker/],
    ["live component has no verified release", () => {}, { deployments: { ...live("bad"), web: { evidenceId: "unverified" } } },
      /no recent verified release recorded/],
    ["runtime config changed", () => {}, { configChanged: () => true }, /runtime config changed/],
    ["untrusted preparation", state => { state.jobs[1].conclusion = "failure"; }, {}, /preparation job/],
    ["receipt names another workflow", state => { state.releaseRun = { path: ".github/workflows/ci.yml" }; }, {},
      /not a successful Release run/],
    ["receipt names an unfinished run", state => { state.releaseRun = { conclusion: "failure" }; }, {},
      /not a successful Release run/],
    ["no earlier release", () => {}, { releases: [release(badSha, "bad", "901")] }, /No earlier verified/],
    ["unknown target", () => {}, { rollbackTo: "f".repeat(40) }, /has no earlier verified/],
    ["receipt without a run", () => {}, { releases: [release(badSha, "bad", "901"), release(sourceSha, "good", null)] },
      /does not name its Release run/],
    ["production already at target", () => {}, { deployments: live("good") }, /already runs/],
  ]) {
    const state = fixture();
    mutate(state);
    await assert.rejects(rollback(state, options), expected, name);
  }
});

test("daily releases pause while a rollback is the newest verified release", async () => {
  const afterRollback = async () => [release(sourceSha, "good", "903", applicationComponents, badSha)];
  const select = (scheduled, loadReleases) => selectRelease({ sourceSha: mainSha, scheduled, repository, loadReleases },
    async () => []);
  assert.equal((await select(true, afterRollback)).paused_after_rollback_of, badSha);
  assert.equal((await select(false, afterRollback)).paused_after_rollback_of, undefined);
  assert.equal((await select(true, async () => [release(mainSha, "next", "904")]))
    .paused_after_rollback_of, undefined);
});
