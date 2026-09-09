import assert from "node:assert/strict";
import test from "node:test";
import { planReceiptRekey, rekeyReceipts } from "./rekey-deployment-receipts.mjs";

const sha = "a".repeat(40);
const newerSha = "b".repeat(40);
function fixture() {
  const deployments = new Map();
  const statuses = new Map();
  let comparison;
  const add = (component, id, sourceSha = sha, date = "2026-09-08T00:00:00Z") => {
    const environment = `production/${component}`;
    const deployment = { id, environment, sha: sourceSha, created_at: date,
      payload: { component, sourceSha, provider: "railway", evidenceId: `railway-${id}` } };
    if (["mediaWorker", "media-worker"].includes(component)) deployment.payload.artifactDigest = `sha256:${"c".repeat(64)}`;
    deployments.set(environment, [deployment, ...(deployments.get(environment) ?? [])]);
    statuses.set(id, [{ state: "success", log_url: `https://github.test/run/${id}` }]);
    return deployment;
  };
  const request = async path => {
    const url = new URL(path, "https://github.test");
    const page = Number(url.searchParams.get("page") ?? 1);
    if (url.pathname === "/deployments") return (deployments.get(url.searchParams.get("environment")) ?? []).slice((page - 1) * 100, page * 100);
    const match = /^\/deployments\/(\d+)\/statuses$/.exec(url.pathname);
    if (match) return (statuses.get(Number(match[1])) ?? []).slice((page - 1) * 100, page * 100);
    if (url.pathname.startsWith("/compare/")) return comparison;
    throw new Error(`Unexpected request ${path}`);
  };
  return { add, deployments, statuses, request, setComparison: value => { comparison = value; } };
}

test("dry run preserves exact source, provider evidence, digest and log URL without writing", async () => {
  const f = fixture();
  f.add("mediaWorker", 1);
  const result = await rekeyReceipts({request: f.request, record: () => { throw new Error("must not write"); }});
  assert.equal(result.apply, false);
  assert.deepEqual(result.plan.find(entry => entry.action === "copy").record, {
    component: "media-worker", sourceSha: sha, provider: "railway", evidenceId: "railway-1",
    artifactDigest: `sha256:${"c".repeat(64)}`, logUrl: "https://github.test/run/1",
  });
});

test("apply writes canonical receipt once and repeated application skips it", async () => {
  const f = fixture();
  f.add("worker", 1);
  const writes = [];
  const record = async value => { writes.push(value); f.add(value.component, 2, value.sourceSha, "2026-09-09T00:00:00Z"); return 2; };
  await rekeyReceipts({apply: true, request: f.request, record});
  await rekeyReceipts({apply: true, request: f.request, record});
  assert.equal(writes.length, 1);
  assert.equal(writes[0].component, "run-worker");
});

test("same revision or newer canonical evidence cannot be overwritten", async () => {
  for (const [sourceSha, date] of [[sha, "2026-09-01T00:00:00Z"], [newerSha, "2026-09-09T00:00:00Z"]]) {
    const f = fixture();
    f.add("router", 1);
    f.add("git-router", 2, sourceSha, date);
    assert.equal((await planReceiptRekey(f.request)).find(entry => entry.component === "git-router").action, "skip");
  }
});

test("source ancestry protects newer canonical revision even with an earlier receipt timestamp", async () => {
  const f = fixture();
  f.add("router", 1);
  f.add("git-router", 2, newerSha, "2026-09-01T00:00:00Z");
  f.setComparison({status: "ahead", base_commit: {sha}, merge_base_commit: {sha}});
  assert.equal((await planReceiptRekey(f.request)).find(entry => entry.component === "git-router").action, "skip");
  f.setComparison({status: "diverged"});
  await assert.rejects(planReceiptRekey(f.request), /Cannot order/);
});

test("malformed source evidence is skipped before copying an older verified receipt", async () => {
  const f = fixture();
  f.add("worker", 1);
  f.add("worker", 2).payload.sourceSha = newerSha;
  const selected = (await planReceiptRekey(f.request)).find(entry => entry.action === "copy");
  assert.equal(selected.historicalId, "1");
});

test("historical receipts and success statuses are paginated", async () => {
  const f = fixture();
  f.add("worker", 1);
  f.statuses.set(1, [...Array.from({length: 100}, () => ({state: "inactive"})), {state: "success"}]);
  const environment = "production/worker";
  f.deployments.set(environment, [...Array.from({length: 100}, (_, i) => ({id: i + 2, environment, payload: {}})), ...f.deployments.get(environment)]);
  assert.equal((await planReceiptRekey(f.request)).find(entry => entry.action === "copy").historicalId, "1");
});

test("both historical and canonical cutover journals must be explicitly terminal", async () => {
  for (const environment of ["production/cutover", "production/maintenance"]) {
    const f = fixture();
    f.deployments.set(environment, [{id: 42, environment, sha, payload: {kind: "scope-release-cutover", prepared: {sourceSha: sha}}}]);
    f.statuses.set(42, [{state: "in_progress", description: "cutover:committed"}]);
    await assert.rejects(planReceiptRekey(f.request), /Unresolved cutover 42/);
    f.statuses.set(42, [{state: "success", description: "cutover:complete"}]);
    await planReceiptRekey(f.request);
  }
});
