import assert from "node:assert/strict";
import test from "node:test";
import { validateRecoveryPreparation } from "./recovery-preparation-trust.mjs";

const repository = "scope-vcs/scope-vcs";
const sourceSha = "a".repeat(40);
const mainSha = "d".repeat(40);
function fixture() {
  const prepared = {
    schemaVersion: 1, sourceSha, preparationRunId: "123", maintenanceSha256: "c".repeat(64),
    components: Object.fromEntries(["api", "worker", "cache", "router"].map(component => [component, {
      sourceSha, serviceId: component,
      image: `ghcr.io/${repository}/railway-private-${component}@sha256:${"b".repeat(64)}`,
    }])),
  };
  const run = {
    id: 123, path: ".github/workflows/scope-production-deploy.yml", event: "push",
    head_branch: "main", head_sha: sourceSha, status: "completed", conclusion: "failure",
    repository: { id: 1, full_name: repository }, head_repository: { id: 1, full_name: repository },
  };
  const main = { name: "main", commit: { sha: mainSha } };
  const comparison = { status: "ahead", base_commit: { sha: sourceSha }, merge_base_commit: { sha: sourceSha } };
  const jobs = [{
    id: 456, run_id: 123, head_sha: sourceSha, name: "Prepare Railway artifacts / prepare",
    status: "completed", conclusion: "success",
    steps: [{ name: "Prepare immutable release images", status: "completed", conclusion: "success" }],
  }];
  const calls = [];
  const request = async path => {
    calls.push(path);
    if (path === "/actions/runs/123") return structuredClone(run);
    if (path === "/branches/main") return structuredClone(main);
    if (path === `/compare/${sourceSha}...${mainSha}`) return structuredClone(comparison);
    const match = /^\/actions\/runs\/123\/jobs\?filter=all&per_page=100&page=(\d+)$/.exec(path);
    if (match) {
      const page = Number(match[1]);
      return { jobs: structuredClone(jobs.slice((page - 1) * 100, page * 100)) };
    }
    throw new Error(`Unexpected request ${path}`);
  };
  return { prepared, run, main, comparison, jobs, calls, request };
}

for (const conclusion of ["failure", "cancelled", "success"]) {
  test(`recovers a main release whose preparation succeeded before overall ${conclusion}`, async () => {
    const state = fixture();
    state.run.conclusion = conclusion;
    const proof = await validateRecoveryPreparation(state.prepared, state.request, repository);
    assert.deepEqual(proof, { sourceSha, preparationRunId: "123", preparationJobId: 456, mainSha });
    assert.equal(state.calls.some(path => /artifacts|logs/.test(path)), false);
  });
}

test("accepts trusted manual production preparation", async () => {
  const state = fixture();
  state.run.event = "workflow_dispatch";
  await validateRecoveryPreparation(state.prepared, state.request, repository);
});

for (const [name, mutate] of [
  ["pull request run", state => { state.run.event = "pull_request"; }],
  ["pull request target run", state => { state.run.event = "pull_request_target"; }],
  ["candidate branch", state => { state.run.head_branch = "candidate"; }],
  ["different source", state => { state.run.head_sha = "e".repeat(40); }],
  ["foreign repository", state => { state.run.repository.full_name = "attacker/repo"; }],
  ["fork head repository", state => { state.run.head_repository.full_name = "attacker/repo"; }],
  ["different repository identity", state => { state.run.head_repository.id = 2; }],
  ["different workflow", state => { state.run.path = ".github/workflows/scope-railway-staging.yml"; }],
  ["different run ID", state => { state.run.id = 124; }],
]) {
  test(`rejects a forged journal pointing at ${name}`, async () => {
    const state = fixture();
    mutate(state);
    await assert.rejects(validateRecoveryPreparation(state.prepared, state.request, repository), /production workflow on main/);
  });
}

for (const status of ["behind", "diverged"]) {
  test(`rejects an unmerged source whose comparison is ${status}`, async () => {
    const state = fixture();
    state.comparison.status = status;
    await assert.rejects(validateRecoveryPreparation(state.prepared, state.request, repository), /trusted main history/);
  });
}

test("requires the source itself to be main's merge base", async () => {
  const state = fixture();
  state.comparison.merge_base_commit.sha = "e".repeat(40);
  await assert.rejects(validateRecoveryPreparation(state.prepared, state.request, repository), /trusted main history/);
});

for (const image of [
  `ghcr.io/scope-vcs/scope-vcs/railway-api@sha256:${"b".repeat(64)}`,
  `ghcr.io/attacker/repo/railway-private-api@sha256:${"b".repeat(64)}`,
  `ghcr.io/scope-vcs/scope-vcs-evil/railway-private-api@sha256:${"b".repeat(64)}`,
  `ghcr.io/scope-vcs/scope-vcs/railway-private-worker@sha256:${"b".repeat(64)}`,
  `registry.example/scope-vcs/scope-vcs/railway-private-api@sha256:${"b".repeat(64)}`,
]) {
  test(`rejects foreign or substituted package ${image.split("@")[0]}`, async () => {
    const state = fixture();
    state.prepared.components.api.image = image;
    await assert.rejects(validateRecoveryPreparation(state.prepared, state.request, repository), /trusted production package/);
    assert.deepEqual(state.calls, []);
  });
}

for (const [name, mutate] of [
  ["failed preparation", job => { job.conclusion = "failure"; }],
  ["unfinished preparation", job => { job.status = "in_progress"; }],
  ["different job", job => { job.name = "Prepare Railway artifacts / forged"; }],
  ["different job source", job => { job.head_sha = "e".repeat(40); }],
  ["different job run", job => { job.run_id = 124; }],
  ["recovery that only reuploaded a manifest", job => { job.steps[0].conclusion = "skipped"; }],
]) {
  test(`rejects ${name}`, async () => {
    const state = fixture();
    mutate(state.jobs[0]);
    await assert.rejects(validateRecoveryPreparation(state.prepared, state.request, repository), /did not successfully build/);
  });
}

test("finds successful preparation from an earlier attempt beyond the first jobs page", async () => {
  const state = fixture();
  state.jobs.unshift(...Array.from({ length: 101 }, () => ({ name: "Later failed job", conclusion: "failure" })));
  await validateRecoveryPreparation(state.prepared, state.request, repository);
  assert.ok(state.calls.includes("/actions/runs/123/jobs?filter=all&per_page=100&page=2"));
});

test("refuses recovery when GitHub cannot establish the original run", async () => {
  const state = fixture();
  await assert.rejects(validateRecoveryPreparation(state.prepared, async () => { throw new Error("GitHub unavailable"); }, repository), /GitHub unavailable/);
});


test("recovery uses the same manifest package prefix as publishing", async () => {
  const state = fixture();
  const manifest = { railway: { releaseImagePrefix: "separate-release-set" } };
  await assert.rejects(validateRecoveryPreparation(state.prepared, state.request, repository, manifest), /trusted production package/);
  assert.deepEqual(state.calls, []);
  for (const artifact of Object.values(state.prepared.components)) {
    artifact.image = artifact.image.replace("/railway-private-", "/separate-release-set-");
  }
  await validateRecoveryPreparation(state.prepared, state.request, repository, manifest);
});
