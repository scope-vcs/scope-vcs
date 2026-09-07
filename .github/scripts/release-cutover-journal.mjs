import { readFileSync, writeFileSync } from "node:fs";
import { validatePreparedRelease } from "./railway-artifact.mjs";

const environment = "production/cutover";
const terminal = new Set(["complete", "restored"]);
function validateCutoverArtifacts(prepared) {
  validatePreparedRelease(prepared, { sourceSha: prepared.sourceSha, components: ["api", "worker", "cache", "router"] });
  if (!/^[a-f0-9]{64}$/.test(prepared.maintenanceSha256 ?? "") || !/^[0-9]+$/.test(prepared.preparationRunId ?? "")) {
    throw new Error("Cutover preparation must pin the maintenance binary digest and its artifact run ID");
  }
}

const phases = new Set([
  "prepared", "closing", "closed", "pre-migration", "applying", "committed",
  "verifying", "backfills", "activating-cache", "activating-worker", "activating-api",
  "reclosing", "restoring", "restored", "complete", "recovery-required",
]);

export async function readCutover(id, request) {
  if (!/^\d+$/.test(String(id))) throw new Error("Cutover ID must be a GitHub deployment ID");
  const deployment = await request(`/deployments/${id}`);
  const payload = typeof deployment.payload === "string" ? JSON.parse(deployment.payload) : deployment.payload;
  if (deployment.environment !== environment || payload?.kind !== "scope-release-cutover"
      || deployment.sha !== payload.prepared?.sourceSha) throw new Error("Invalid cutover deployment");
  validateCutoverArtifacts(payload.prepared);
  if (!Array.isArray(payload.baseline?.pending) || !payload.previous) throw new Error("Invalid cutover baseline");
  const statuses = [];
  for (let page = 1; ; page += 1) {
    const batch = await request(`/deployments/${id}/statuses?per_page=100&page=${page}`);
    statuses.push(...batch);
    if (batch.length < 100) break;
  }
  const events = statuses.map((status) => {
    const match = /^cutover:([a-z-]+)$/.exec(status.description ?? "");
    if (!match || !phases.has(match[1])) throw new Error("Unknown cutover journal status");
    return { phase: match[1], at: status.created_at };
  });
  // The deployment itself is the durable closure intent if cancellation preceded the first status.
  return { id: String(id), ...payload, phase: events[0]?.phase ?? "prepared", events };
}

export async function guardCutovers(request, recoveryId = "") {
  for (let page = 1; ; page += 1) {
    const deployments = await request(`/deployments?environment=${encodeURIComponent(environment)}&per_page=100&page=${page}`);
    for (const deployment of deployments) {
      const journal = await readCutover(deployment.id, request);
      if (!terminal.has(journal.phase) && journal.id !== String(recoveryId)) {
        throw new Error(`Unresolved cutover ${journal.id} from ${journal.prepared.sourceSha}; recover that release before deploying`);
      }
    }
    if (deployments.length < 100) return;
  }
}

export async function recordCutoverPhase(id, phase, request) {
  if (!phases.has(phase)) throw new Error(`Invalid cutover phase: ${phase}`);
  const journal = await readCutover(id, request);
  if (terminal.has(journal.phase)) throw new Error(`Cutover ${id} is already ${journal.phase}`);
  await request(`/deployments/${id}/statuses`, {
    method: "POST", headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ state: terminal.has(phase) ? "success" : "in_progress", environment,
      auto_inactive: false, description: `cutover:${phase}` }),
  });
}

export async function beginCutover({ prepared, baseline, previous }, request) {
  validateCutoverArtifacts(prepared);
  if (!Array.isArray(baseline?.pending)) throw new Error("Invalid baseline migration plan");
  await guardCutovers(request);
  const deployment = await request("/deployments", {
    method: "POST", headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ ref: prepared.sourceSha, auto_merge: false, required_contexts: [],
      environment, production_environment: true, transient_environment: false,
      description: `Cutover ${prepared.sourceSha.slice(0, 12)}`,
      payload: { kind: "scope-release-cutover", prepared, baseline, previous } }),
  });
  await recordCutoverPhase(deployment.id, "prepared", request);
  return String(deployment.id);
}

export async function cutoverCommand(command, argument, request) {
  const id = argument("--id");
  const json = (path) => JSON.parse(readFileSync(path, "utf8"));
  if (command === "cutover-guard") return guardCutovers(request, id);
  if (command === "cutover-begin") {
    return beginCutover({ prepared: json(argument("--manifest")), baseline: json(argument("--baseline")), previous: json(argument("--previous")) }, request);
  }
  if (command === "cutover-phase") return recordCutoverPhase(id, argument("--phase"), request);
  if (command === "cutover-restore" || command === "cutover-read") {
    await guardCutovers(request, id);
    const journal = await readCutover(id, request);
    if (terminal.has(journal.phase)) throw new Error(`Cutover ${id} is already ${journal.phase}`);
    if (journal.prepared.sourceSha !== argument("--source-sha")) throw new Error("Recovery source SHA must match the pinned cutover revision");
    if (command === "cutover-restore") writeFileSync(argument("--manifest"), `${JSON.stringify(journal.prepared, null, 2)}\n`);
    return journal;
  }
  throw new Error(`Unknown cutover command: ${command}`);
}
