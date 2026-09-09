#!/usr/bin/env node

// One-time migration of deployment evidence names. This is not release runtime code.
import { pathToFileURL } from "node:url";
import { githubRequest, recordSuccessfulDeployment } from "../../.github/scripts/production-deployment-progress.mjs";

const renames = {
  worker: "run-worker", router: "git-router", media: "media-api",
  mediaWorker: "media-worker", cli: "cli-downloads", checksImage: "checks-image",
};
const shaPattern = /^[0-9a-f]{40}$/;
const digestPattern = /^sha256:[0-9a-f]{64}$/;

async function* pages(path, request) {
  for (let page = 1; ; page += 1) {
    const batch = await request(`${path}${path.includes("?") ? "&" : "?"}per_page=100&page=${page}`);
    if (!Array.isArray(batch)) throw new Error(`Invalid GitHub response for ${path}`);
    yield* batch;
    if (batch.length < 100) return;
  }
}

function payloadOf(deployment) {
  try { return typeof deployment.payload === "string" ? JSON.parse(deployment.payload) : deployment.payload; }
  catch { return null; }
}

async function guardHistoricalCutovers(request) {
  for (const environment of ["production/cutover", "production/maintenance"]) {
    for await (const deployment of pages(`/deployments?environment=${encodeURIComponent(environment)}`, request)) {
      const payload = payloadOf(deployment);
      if (deployment.environment !== environment || payload?.kind !== "scope-release-cutover"
          || !shaPattern.test(deployment.sha ?? "") || payload.prepared?.sourceSha !== deployment.sha) {
        throw new Error(`Cannot verify historical cutover ${deployment.id}`);
      }
      const statuses = await request(`/deployments/${deployment.id}/statuses?per_page=100&page=1`);
      if (!Array.isArray(statuses) || !["cutover:complete", "cutover:restored"].includes(statuses[0]?.description)
          || statuses[0]?.state !== "success") {
        throw new Error(`Unresolved cutover ${deployment.id}; recover before rekeying receipts`);
      }
    }
  }
}

async function latestReceipt(component, request) {
  const environment = `production/${component}`;
  for await (const deployment of pages(`/deployments?environment=${encodeURIComponent(environment)}`, request)) {
    const payload = payloadOf(deployment);
    if (deployment.environment !== environment || payload?.component !== component
        || payload.sourceSha !== deployment.sha || !shaPattern.test(deployment.sha ?? "")
        || typeof payload.provider !== "string" || !payload.provider
        || typeof payload.evidenceId !== "string" || !payload.evidenceId
        || (payload.artifactDigest !== undefined && !digestPattern.test(payload.artifactDigest))
        || (["mediaWorker", "media-worker"].includes(component) && !digestPattern.test(payload.artifactDigest ?? ""))) continue;
    for await (const status of pages(`/deployments/${deployment.id}/statuses`, request)) {
      if (status.state !== "success") continue;
      const createdAt = Date.parse(deployment.created_at);
      if (!Number.isFinite(createdAt)) throw new Error(`Receipt ${deployment.id} has no valid creation timestamp`);
      return { id: String(deployment.id), createdAt, record: {
        component, sourceSha: payload.sourceSha, provider: payload.provider, evidenceId: payload.evidenceId,
        ...(payload.artifactDigest ? { artifactDigest: payload.artifactDigest } : {}),
        ...(status.log_url ? { logUrl: status.log_url } : {}),
      } };
    }
  }
  return null;
}

export async function planReceiptRekey(request) {
  await guardHistoricalCutovers(request);
  const plan = [];
  for (const [oldComponent, component] of Object.entries(renames)) {
    const previous = await latestReceipt(oldComponent, request);
    if (!previous) { plan.push({ component, action: "skip", reason: "No valid historical receipt" }); continue; }
    const current = await latestReceipt(component, request);
    if (current) {
      if (current.createdAt >= previous.createdAt || current.record.sourceSha === previous.record.sourceSha) {
        plan.push({ component, action: "skip", reason: "Canonical receipt is the same revision or newer", canonicalId: current.id });
        continue;
      }
      const comparison = await request(`/compare/${previous.record.sourceSha}...${current.record.sourceSha}`);
      if (["ahead", "identical"].includes(comparison.status) && comparison.base_commit?.sha === previous.record.sourceSha
          && comparison.merge_base_commit?.sha === previous.record.sourceSha) {
        plan.push({ component, action: "skip", reason: "Canonical revision is newer", canonicalId: current.id });
        continue;
      }
      if (comparison.status !== "behind" || comparison.base_commit?.sha !== previous.record.sourceSha
          || comparison.merge_base_commit?.sha !== current.record.sourceSha) {
        throw new Error(`Cannot order historical and canonical revisions for ${component}`);
      }
    }
    plan.push({ component, action: "copy", historicalId: previous.id, record: { ...previous.record, component } });
  }
  return plan;
}

export async function rekeyReceipts({ apply = false, request = githubRequest, record = recordSuccessfulDeployment } = {}) {
  const plan = await planReceiptRekey(request);
  if (apply) {
    // Refresh each candidate immediately before writing. A release must not run concurrently.
    for (const candidate of plan.filter(entry => entry.action === "copy")) {
      const refreshed = (await planReceiptRekey(request)).find(entry => entry.component === candidate.component);
      if (refreshed.action !== "copy") continue;
      if (JSON.stringify(refreshed.record) !== JSON.stringify(candidate.record)) throw new Error("Receipt evidence changed; rerun the migration");
      candidate.deploymentId = await record(candidate.record);
    }
  }
  return { apply, plan };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  if (process.argv.slice(2).some(argument => argument !== "--apply")) throw new Error("Usage: rekey-deployment-receipts.mjs [--apply]");
  rekeyReceipts({ apply: process.argv.includes("--apply") })
    .then(result => process.stdout.write(`${JSON.stringify(result, null, 2)}\n`))
    .catch(error => { console.error(error.message); process.exitCode = 1; });
}
