import assert from "node:assert/strict";
import test from "node:test";
import { beginCutover, guardCutovers, readCutover, recordCutoverPhase, cutoverCommand } from "./release-cutover-journal.mjs";

const sourceSha = "a".repeat(40);
const prepared = { schemaVersion: 1, sourceSha, maintenanceSha256: "c".repeat(64), preparationRunId: "123", components: Object.fromEntries(
  ["api", "worker", "cache", "router"].map(component => [component, {
    serviceId: component, sourceSha, image: `ghcr.io/scope/${component}@sha256:${"b".repeat(64)}`,
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
