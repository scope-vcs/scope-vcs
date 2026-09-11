import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { validatePreparedDeployment, selectRelease } from "./release-selection.mjs";
import { validateRecoveryPreparation } from "./recovery-preparation-trust.mjs";

import { repository, sourceSha, backendComponents, releaseFixture } from "./fixtures/prepared-release.mjs";
const sourceRunId = "34281642523";
const mainSha = "b".repeat(40);
const manifest = JSON.parse(readFileSync(new URL("../deployment-services.json", import.meta.url)));

function fixture() {
  const state = releaseFixture({ sourceRunId, mainSha, services: manifest.services,
    components: [...backendComponents, 'web'] });
  state.run.event = 'workflow_dispatch';
  const successful = state.jobs[0];
  state.jobs.unshift({ ...successful, id: 1, name: 'Validate selected components / Production validation gate', steps: [] });
  state.jobs.push({ ...successful, id: 3, name: 'Deploy staging / Deploy and smoke staging', steps: [] });
  return state;
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
  state.prepared.components = { web: state.prepared.components.web };
  await assert.rejects(validateRecoveryPreparation(state.prepared, state.request, repository, manifest), /missing cache/);
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
  assert.equal(result.recover_components.backend, true);
  assert.deepEqual(result.prepared, state.prepared);
});

test("web-only replay does not select the backend", async () => {
  const state = fixture();
  state.prepared.components = { web: state.prepared.components.web };
  const result = await selectRelease({ sourceSha: mainSha, sourceRunId, repository,
    loadPrepared: async () => state.prepared }, path => path.startsWith("/deployments?") ? [] : state.request(path));
  assert.equal(result.recover_components.web, true);
  assert.equal(result.recover_components.backend, false);
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
