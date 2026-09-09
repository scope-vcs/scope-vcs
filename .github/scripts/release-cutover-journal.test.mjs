import assert from "node:assert/strict";
import test from "node:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { beginCutover, guardCutovers, readCutover, recordCutoverPhase, cutoverCommand } from "./release-cutover-journal.mjs";

const sourceSha = "a".repeat(40);
const prepared = { schemaVersion: 1, sourceSha, maintenanceSha256: "c".repeat(64), preparationRunId: "123", components: Object.fromEntries(
  ["api", "worker", "cache", "router", "media", "mediaWorker"].map(component => [component, {
    serviceId: component, sourceSha, image: component === "mediaWorker"
      ? `ghcr.io/scope-vcs/scope-media-worker@sha256:${"b".repeat(64)}`
      : `ghcr.io/scope-vcs/scope-vcs/railway-private-${component}@sha256:${"b".repeat(64)}`,
  }]),
) };
const baseline = { exact: false, pending: [{ name: "m0033", impact: "maintenance-required" }] };

function storage() {
  const deployments = [];
  const statuses = new Map();
  const request = async (path, options = {}) => {
    const body = options.body ? JSON.parse(options.body) : null;
    if (path === "/deployments" && body) {
      const deployment = { ...body, sha: body.ref, id: deployments.length + 1 };
      deployments.unshift(deployment);
      statuses.set(String(deployment.id), []);
      return structuredClone(deployment);
    }
    const page = Number(new URL(path, "https://github.test").searchParams.get("page") || 1);
    if (path.startsWith("/deployments?")) return structuredClone(deployments.slice((page - 1) * 100, page * 100));
    const [, id, suffix] = /^\/deployments\/(\d+)(.*)$/.exec(path);
    if (!suffix) return structuredClone(deployments.find(d => String(d.id) === id));
    if (body) statuses.get(id).unshift({ ...body, created_at: new Date().toISOString() });
    return structuredClone(statuses.get(id).slice((page - 1) * 100, page * 100));
  };
  return { request, deployments, statuses };
}

test("durable intent blocks a later release even if the runner dies before its first phase", async () => {
  const store = storage();
  const id = await beginCutover({ prepared, baseline, previous: {} }, store.request);
  store.statuses.set(id, []);
  await assert.rejects(guardCutovers(store.request), /Unresolved cutover/);
  await guardCutovers(store.request, id);
  assert.equal((await readCutover(id, store.request)).phase, "prepared");
});

test("applying intent and artifact identities survive a fresh reader after runner loss", async () => {
  const { request } = storage();
  const id = await beginCutover({ prepared, baseline, previous: { api: { evidenceId: "old-api" } } }, request);
  await recordCutoverPhase(id, "applying", request);
  const recovered = await readCutover(id, request);
  assert.equal(recovered.phase, "applying");
  assert.deepEqual(recovered.prepared, prepared);
  assert.deepEqual(recovered.baseline, baseline);
  assert.equal(recovered.previous.api.evidenceId, "old-api");
  await assert.rejects(cutoverCommand("cutover-read", name => ({ "--id": id, "--source-sha": "c".repeat(40) })[name], request), /source SHA/);
});

test("only completed forward activation or verified restoration clears the guard", async () => {
  for (const terminal of ["complete", "restored"]) {
    const { request } = storage();
    const id = await beginCutover({ prepared, baseline, previous: {} }, request);
    await recordCutoverPhase(id, "recovery-required", request);
    await assert.rejects(guardCutovers(request), /Unresolved/);
    await recordCutoverPhase(id, terminal, request);
    await guardCutovers(request);
    await assert.rejects(recordCutoverPhase(id, "closing", request), /already/);
  }
});

test("incomplete preparation fails before creating closure intent", async () => {
  const store = storage();
  const incomplete = structuredClone(prepared);
  delete incomplete.components.cache;
  await assert.rejects(beginCutover({ prepared: incomplete, baseline, previous: {} }, store.request));
  assert.equal(store.deployments.length, 0);
});

test("unknown durable status and unreadable journals fail closed", async () => {
  const store = storage();
  const id = await beginCutover({ prepared, baseline, previous: {} }, store.request);
  store.statuses.get(id).unshift({ description: "unknown" });
  await assert.rejects(guardCutovers(store.request), /Unknown/);
  await assert.rejects(guardCutovers(async () => { throw new Error("offline"); }), /offline/);
});

test("recovery retains the original closure timestamp after more than one status page", async () => {
  const store = storage();
  const id = await beginCutover({ prepared, baseline, previous: {} }, store.request);
  store.statuses.set(id, [
    ...Array.from({length: 101}, () => ({description: "cutover:reclosing", created_at: "2026-09-07T00:00:00Z"})),
    {description: "cutover:closing", created_at: "2026-09-06T00:00:00Z"},
  ]);
  const journal = await readCutover(id, store.request);
  assert.equal(journal.events.length, 102);
  assert.equal(journal.events.at(-1).at, "2026-09-06T00:00:00Z");
});

function recoveryRequest(store, event = "schedule") {
  return async (path, options) => {
    if (path === "/actions/runs/123") return {
      id: 123, path: ".github/workflows/scope-production-deploy.yml", event,
      head_branch: "main", head_sha: sourceSha, conclusion: "cancelled",
      repository: { id: 1, full_name: "scope-vcs/scope-vcs" },
      head_repository: { id: 1, full_name: "scope-vcs/scope-vcs" },
    };
    if (path === "/branches/main") return { name: "main", commit: { sha: sourceSha } };
    if (path === `/compare/${sourceSha}...${sourceSha}`) return {
      status: "identical", base_commit: { sha: sourceSha }, merge_base_commit: { sha: sourceSha },
    };
    if (path.startsWith("/actions/runs/123/jobs?")) return { jobs: [{
      id: 456, run_id: 123, head_sha: sourceSha, name: "Prepare Railway artifacts / prepare",
      status: "completed", conclusion: "success",
      steps: [{ name: "Prepare immutable release images", conclusion: "success" }],
    }] };
    return store.request(path, options);
  };
}

for (const command of ["cutover-read", "cutover-restore", "cutover-validate-recovery"]) {
  test(`${command} rejects a forged PR-run journal before returning or writing it`, async () => {
    const store = storage();
    const id = await beginCutover({ prepared, baseline, previous: {} }, store.request);
    const directory = mkdtempSync(join(tmpdir(), "cutover-trust-"));
    const manifest = join(directory, "prepared.json");
    writeFileSync(manifest, "existing trusted manifest");
    try {
      await assert.rejects(cutoverCommand(command, name => ({
        "--id": id, "--source-sha": sourceSha, "--manifest": manifest,
      })[name], recoveryRequest(store, "pull_request"), { repository: "scope-vcs/scope-vcs" }), /production workflow on main/);
      assert.equal(readFileSync(manifest, "utf8"), "existing trusted manifest");
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  });
}

test("validated restore retains a successful preparation from a cancelled production run", async () => {
  const store = storage();
  const id = await beginCutover({ prepared, baseline, previous: {} }, store.request);
  const directory = mkdtempSync(join(tmpdir(), "cutover-trust-"));
  const manifest = join(directory, "prepared.json");
  try {
    const result = await cutoverCommand("cutover-restore", name => ({
      "--id": id, "--source-sha": sourceSha, "--manifest": manifest,
    })[name], recoveryRequest(store), { repository: "scope-vcs/scope-vcs" });
    assert.deepEqual(JSON.parse(readFileSync(manifest, "utf8")), prepared);
    assert.equal(result.trustedPreparation.preparationJobId, 456);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

test("payload fields cannot override the GitHub deployment identity", async () => {
  const store = storage();
  const id = await beginCutover({ prepared, baseline, previous: {} }, store.request);
  store.deployments[0].payload.id = "999";
  assert.equal((await readCutover(id, store.request)).id, id);
});
