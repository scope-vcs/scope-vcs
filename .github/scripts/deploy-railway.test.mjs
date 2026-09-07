import assert from "node:assert/strict";
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";

const deployScript = fileURLToPath(new URL("./deploy-railway.sh", import.meta.url));

function providerStatus(activeDeployments) {
  return {
    environments: { edges: [{ node: {
      id: "staging-id",
      name: "staging",
      serviceInstances: { edges: [{ node: {
        serviceId: "cache-id",
        serviceName: "scope-cache-service",
        latestDeployment: { id: "stale-failed", status: "FAILED" },
        activeDeployments,
      } }] },
    } }] },
  };
}

function deploy(t, status, predecessors = []) {
  const root = mkdtempSync(join(tmpdir(), "scope-railway-teardown-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const scripts = join(root, ".github/scripts");
  const bin = join(root, "bin");
  mkdirSync(scripts, { recursive: true });
  mkdirSync(bin);
  writeFileSync(join(root, "status.json"), JSON.stringify(status));
  writeFileSync(join(root, "prepared.json"), JSON.stringify({
    components: { cache: { serviceId: "cache-id" } },
  }));
  writeFileSync(join(scripts, "railway-artifact.mjs"), `
    import { appendFileSync } from "node:fs";
    const command = process.argv[2];
    appendFileSync("events", command + "\\n");
    if (command === "activate") console.log(JSON.stringify({ deploymentId: "new-cache" }));
  `);
  writeFileSync(join(bin, "railway"), `#!/usr/bin/env node
    const { appendFileSync, existsSync, readFileSync, writeFileSync } = require("node:fs");
    const command = process.argv.slice(2, 4).join(" ");
    appendFileSync("events", command + "\\n");
    if (command === "service list") {
      console.log(JSON.stringify([{ id: "cache-id", deploymentId: "stale-failed", status: "FAILED" }]));
    } else if (process.argv[2] === "status") {
      console.log(readFileSync("status.json", "utf8"));
    } else if (command === "deployment list") {
      const count = (existsSync("polls") ? Number(readFileSync("polls", "utf8")) : 0) + 1;
      writeFileSync("polls", String(count));
      if (count > 10) process.exit(1);
      console.log(JSON.stringify([
        { id: "new-cache", status: "SUCCESS" },
        { id: "stale-failed", status: "FAILED" },
        ...${JSON.stringify(predecessors)}.map((id, index) => ({
          id, status: count >= 4 + index * 2 ? "REMOVED" : "SUCCESS",
        })),
      ]));
    } else { process.exit(1); }
  `, { mode: 0o755 });
  writeFileSync(join(bin, "sleep"), "#!/bin/sh\nprintf 'sleep\\n' >> events\n", { mode: 0o755 });
  const result = spawnSync("bash", [deployScript, "cache-id", root], {
    cwd: root,
    encoding: "utf8",
    timeout: 10_000,
    env: {
      ...process.env,
      PATH: `${bin}:${process.env.PATH}`,
      RAILWAY_API_TOKEN: "test-token",
      RAILWAY_TOKEN: "",
      RAILWAY_PROJECT_ID: "test-project",
      SCOPE_RAILWAY_ENVIRONMENT_ID: "staging-id",
      SCOPE_DEPLOYMENT_COMPONENT: "cache",
      SCOPE_DEPLOYMENT_SOURCE_SHA: "a".repeat(40),
      SCOPE_PREPARED_RELEASE_PATH: join(root, "prepared.json"),
      SCOPE_DEFER_SERVICE_HEALTH: "1",
      SCOPE_DEPLOYMENT_EVIDENCE_PATH: "",
      SCOPE_RELEASE_DEPLOYMENTS_FILE: "",
    },
  });
  assert.ifError(result.error);
  return { ...result, events: readFileSync(join(root, "events"), "utf8").trim().split("\n") };
}

test("stale FAILED latest deployment without active predecessors needs no teardown", (t) => {
  const result = deploy(t, providerStatus([]));
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.events.filter((event) => event === "deployment list").length, 2);
  assert.ok(!result.events.includes("sleep"));
  assert.ok(result.events.indexOf("status --project") < result.events.indexOf("activate"));
});

test("waits for the actual active predecessor even when latest deployment is FAILED", (t) => {
  const result = deploy(t, providerStatus([{ id: "old-cache", status: "SUCCESS" }]), ["old-cache"]);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.events.filter((event) => event === "sleep").length, 1);
  assert.match(result.stdout, /Previous deployment old-cache completed teardown/);
});

test("waits for every active predecessor to finish teardown", (t) => {
  const predecessors = ["old-cache-1", "old-cache-2"];
  const result = deploy(t, providerStatus(predecessors.map((id) => ({ id, status: "SUCCESS" }))), predecessors);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.events.filter((event) => event === "sleep").length, 2);
  for (const id of predecessors) assert.ok(result.stdout.includes(`Previous deployment ${id} completed teardown.`));
});

test("missing or malformed provider state fails before activation", (t) => {
  const missingService = providerStatus([]);
  missingService.environments.edges[0].node.serviceInstances.edges = [];
  for (const status of [{}, missingService, providerStatus(undefined), providerStatus(null), providerStatus([{}]), providerStatus([{ id: "" }])]) {
    const result = deploy(t, status);
    assert.notEqual(result.status, 0);
    assert.ok(!result.events.includes("activate"));
  }
});
