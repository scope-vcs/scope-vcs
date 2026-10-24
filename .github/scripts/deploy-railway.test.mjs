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
        serviceId: "router-id",
        serviceName: "scope-git-router",
        latestDeployment: { id: "stale-failed", status: "FAILED" },
        activeDeployments,
      } }] },
    } }] },
  };
}

function deploy(t, status, predecessors = [], failedPolls = [], component = "git-router", defer = false) {
  const root = mkdtempSync(join(tmpdir(), "scope-railway-teardown-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const scripts = join(root, ".github/scripts");
  const bin = join(root, "bin");
  mkdirSync(scripts, { recursive: true });
  mkdirSync(bin);
  if (defer) mkdirSync(join(root, 'predecessors'));
  for (const name of ['railway-predecessor-teardown.mjs', 'railway-read.mjs', 'railway-retry.mjs']) {
    writeFileSync(join(scripts, name), readFileSync(new URL(`./${name}`, import.meta.url)));
  }
  writeFileSync(join(scripts, "deployment-components.mjs"), readFileSync(new URL("./deployment-components.mjs", import.meta.url)));
  writeFileSync(join(root, ".github/deployment-services.json"), readFileSync(new URL("../deployment-services.json", import.meta.url)));
  writeFileSync(join(root, "status.json"), JSON.stringify(status));
  writeFileSync(join(root, "prepared.json"), JSON.stringify({
    components: { [component]: { serviceId: "router-id" } },
  }));
  writeFileSync(join(scripts, "railway-artifact.mjs"), `
    import { appendFileSync } from "node:fs";
    const command = process.argv[2];
    appendFileSync("events", command + "\\n");
    if (command === "activate") console.log(JSON.stringify({ deploymentId: "new-router" }));
  `);
  writeFileSync(join(bin, "railway"), `#!/usr/bin/env node
    const { appendFileSync, existsSync, readFileSync, writeFileSync } = require("node:fs");
    const command = process.argv.slice(2, 4).join(" ");
    appendFileSync("events", command + "\\n");
    if (command === "service list") {
      console.log(JSON.stringify([{ id: "router-id", deploymentId: "stale-failed", status: "FAILED" }]));
    } else if (process.argv[2] === "status") {
      console.log(readFileSync("status.json", "utf8"));
    } else if (command === "deployment list") {
      const count = (existsSync("polls") ? Number(readFileSync("polls", "utf8")) : 0) + 1;
      writeFileSync("polls", String(count));
      if (${JSON.stringify(failedPolls)}.includes(count)) {
        console.log("incomplete provider response");
        console.error("provider read timed out");
        process.exit(1);
      }
      if (count > 10) process.exit(1);
      console.log(JSON.stringify([
        { id: "new-router", status: "SUCCESS" },
        { id: "stale-failed", status: "FAILED" },
        ...${JSON.stringify(predecessors)}.map((id, index) => ({
          id, status: count >= 4 + index * 2 ? "REMOVED" : "SUCCESS",
        })),
      ]));
    } else { process.exit(1); }
  `, { mode: 0o755 });
  writeFileSync(join(bin, "sleep"), "#!/bin/sh\nprintf 'sleep\\n' >> events\n", { mode: 0o755 });
  const result = spawnSync("bash", [deployScript, "router-id"], {
    cwd: root,
    encoding: "utf8",
    timeout: 60_000,
    env: {
      ...process.env,
      PATH: `${bin}:${process.env.PATH}`,
      RAILWAY_API_TOKEN: "test-token",
      RAILWAY_TOKEN: "",
      RAILWAY_PROJECT_ID: "test-project",
      SCOPE_RAILWAY_ENVIRONMENT_ID: "staging-id",
      SCOPE_DEPLOYMENT_COMPONENT: component,
      SCOPE_DEPLOYMENT_SOURCE_SHA: "a".repeat(40),
      SCOPE_PREPARED_RELEASE_PATH: join(root, "prepared.json"),
      SCOPE_DEFER_SERVICE_HEALTH: "1",
      SCOPE_DEPLOYMENT_EVIDENCE_PATH: "",
      SCOPE_RELEASE_DEPLOYMENTS_FILE: "",
      SCOPE_PREDECESSOR_TEARDOWN_DIR: defer ? join(root, 'predecessors') : '',
      SCOPE_PREDECESSOR_TEARDOWN_POLL_SECONDS: "0",
      SCOPE_RAILWAY_READ_RETRY_DELAY_MS: "0",
    },
  });
  assert.ifError(result.error);
  return { ...result, events: readFileSync(join(root, "events"), "utf8").trim().split("\n"),
    snapshot: defer ? JSON.parse(readFileSync(join(root, 'predecessors', `${component}.json`))) : null };
}

test("stale FAILED latest deployment without active predecessors needs no teardown", (t) => {
  const result = deploy(t, providerStatus([]));
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.events.filter((event) => event === "deployment list").length, 2);
  assert.ok(result.events.indexOf("status --project") < result.events.indexOf("activate"));
});

test("waits for the actual active predecessor even when latest deployment is FAILED", (t) => {
  const result = deploy(t, providerStatus([{ id: "old-router", status: "SUCCESS" }]), ["old-router"]);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.events.filter((event) => event === "deployment list").length, 4);
  assert.match(result.stdout, /Previous deployment old-router completed teardown/);
});

test("waits for every active predecessor to finish teardown", (t) => {
  const predecessors = ["old-router-1", "old-router-2"];
  const result = deploy(t, providerStatus(predecessors.map((id) => ({ id, status: "SUCCESS" }))), predecessors);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.events.filter((event) => event === "deployment list").length, 6);
  for (const id of predecessors) assert.ok(result.stdout.includes(`Previous deployment ${id} completed teardown.`));
});

test("deferred activation records predecessors before mutation and leaves removal to the shared barrier", (t) => {
  const result = deploy(t, providerStatus([{ id: 'old-router', status: 'SUCCESS' }]), ['old-router'], [], 'git-router', true);
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.snapshot, { component: 'git-router', service: 'router-id', ids: ['old-router'] });
  assert.equal(result.events.filter((event) => event === 'deployment list').length, 2);
  assert.ok(result.events.indexOf('status --project') < result.events.indexOf('activate'));
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

test("retries metadata and teardown reads without repeating activation", (t) => {
  const result = deploy(t, providerStatus([{ id: "old-router", status: "SUCCESS" }]), ["old-router"], [2, 4]);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.events.filter((event) => event === "activate").length, 1);
  assert.equal(result.events.filter((event) => event === "verify").length, 1);
  assert.match(result.stdout, /Previous deployment old-router completed teardown/);
  assert.ok(!result.stdout.includes("incomplete provider response"));
});

test("fails after three unsuccessful metadata reads without repeating activation", (t) => {
  const result = deploy(t, providerStatus([]), [], [2, 3, 4]);
  assert.notEqual(result.status, 0);
  assert.equal(result.events.filter((event) => event === "deployment list").length, 4);
  assert.equal(result.events.filter((event) => event === "activate").length, 1);
  assert.ok(!result.events.includes("verify"));
  assert.match(result.stderr, /Railway read failed after 3 attempts/);
});

test("prepared CLI image activates and verifies without requiring backend transition settings", (t) => {
  const result = deploy(t, providerStatus([]), [], [], "cli-downloads");
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.events.filter((event) => event === "activate").length, 1);
  assert.equal(result.events.filter((event) => event === "verify").length, 1);
  assert.ok(!result.events.some((event) => event.startsWith("up ")));
});
