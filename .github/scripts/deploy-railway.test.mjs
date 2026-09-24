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
        serviceName: "scope-cache",
        latestDeployment: { id: "stale-failed", status: "FAILED" },
        activeDeployments,
      } }] },
    } }] },
  };
}

function deploy(t, status, predecessors = [], failedPolls = [], component = "cache", defer = false) {
  const root = mkdtempSync(join(tmpdir(), "scope-railway-teardown-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const scripts = join(root, ".github/scripts");
  const bin = join(root, "bin");
  mkdirSync(scripts, { recursive: true });
  mkdirSync(bin);
  if (defer) {
    mkdirSync(join(root, 'predecessors'));
    writeFileSync(join(scripts, 'railway-predecessor-teardown.mjs'), readFileSync(new URL('./railway-predecessor-teardown.mjs', import.meta.url)));
    writeFileSync(join(scripts, 'railway-read.mjs'), readFileSync(new URL('./railway-read.mjs', import.meta.url)));
  }
  writeFileSync(join(scripts, "deployment-components.mjs"), readFileSync(new URL("./deployment-components.mjs", import.meta.url)));
  writeFileSync(join(root, ".github/deployment-services.json"), readFileSync(new URL("../deployment-services.json", import.meta.url)));
  writeFileSync(join(root, "status.json"), JSON.stringify(status));
  writeFileSync(join(root, "prepared.json"), JSON.stringify({
    components: { [component]: { serviceId: "cache-id" } },
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
      if (${JSON.stringify(failedPolls)}.includes(count)) {
        console.log("incomplete provider response");
        console.error("provider read timed out");
        process.exit(1);
      }
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
  const result = spawnSync("bash", [deployScript, "cache-id"], {
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
      SCOPE_DEPLOYMENT_COMPONENT: component,
      SCOPE_DEPLOYMENT_SOURCE_SHA: "a".repeat(40),
      SCOPE_PREPARED_RELEASE_PATH: join(root, "prepared.json"),
      SCOPE_DEFER_SERVICE_HEALTH: "1",
      SCOPE_DEPLOYMENT_EVIDENCE_PATH: "",
      SCOPE_RELEASE_DEPLOYMENTS_FILE: "",
      SCOPE_PREDECESSOR_TEARDOWN_DIR: defer ? join(root, 'predecessors') : '',
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

test("deferred activation records predecessors before mutation and leaves removal to the shared barrier", (t) => {
  const result = deploy(t, providerStatus([{ id: 'old-cache', status: 'SUCCESS' }]), ['old-cache'], [], 'cache', true);
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.snapshot, { component: 'cache', service: 'cache-id', ids: ['old-cache'] });
  assert.equal(result.events.filter((event) => event === 'sleep').length, 0);
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
  const result = deploy(t, providerStatus([{ id: "old-cache", status: "SUCCESS" }]), ["old-cache"], [2, 4]);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.events.filter((event) => event === "activate").length, 1);
  assert.equal(result.events.filter((event) => event === "verify").length, 1);
  assert.match(result.stdout, /Previous deployment old-cache completed teardown/);
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

function upload(t, { output, exitCode, stderr = "" }) {
  const root = mkdtempSync(join(tmpdir(), "scope-railway-upload-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const bin = join(root, "bin");
  mkdirSync(bin);
  writeFileSync(join(bin, "railway"), `#!/usr/bin/env node
    const { appendFileSync } = require("node:fs");
    const command = process.argv[2];
    appendFileSync("events", command + "\\n");
    if (command === "service") console.log(JSON.stringify([{ id: "cli-id" }]));
    else if (command === "up") {
      console.log(${JSON.stringify(output)});
      console.error(${JSON.stringify(stderr)});
      process.exit(${exitCode});
    } else if (command === "deployment") console.log(JSON.stringify([{ id: "cli-deploy", status: "SUCCESS" }]));
    else process.exit(99);
  `, { mode: 0o755 });
  const result = spawnSync("bash", [deployScript, "cli-id", "upload-root"], {
    cwd: root, encoding: "utf8", timeout: 10_000,
    env: {
      ...process.env, PATH: `${bin}:${process.env.PATH}`,
      RAILWAY_API_TOKEN: "test-token", RAILWAY_TOKEN: "", RAILWAY_PROJECT_ID: "test-project",
      SCOPE_RAILWAY_ENVIRONMENT_ID: "production-id", SCOPE_PREPARED_RELEASE_PATH: "",
      SCOPE_DEFER_SERVICE_HEALTH: "1", SCOPE_DEPLOYMENT_EVIDENCE_PATH: "", SCOPE_RELEASE_DEPLOYMENTS_FILE: "",
    },
  });
  assert.ifError(result.error);
  return { ...result, events: readFileSync(join(root, "events"), "utf8").trim().split("\n") };
}

test("source-upload failures retain status diagnostics without leaking signed URLs or retrying", (t) => {
  const output = JSON.stringify({ statusCode: 502, error: "https://provider.invalid/upload?token=do-not-print" });
  const result = upload(t, { output, exitCode: 7, stderr: "sensitive stderr token=also-do-not-print" });
  assert.equal(result.status, 7);
  assert.match(result.stderr, /source upload failed \(exit 7; HTTP 502\)/);
  assert.match(result.stderr, /No deployment receipt was returned/);
  assert.doesNotMatch(result.stdout + result.stderr, /do-not-print|provider\.invalid|sensitive stderr/);
  assert.deepEqual(result.events, ["service", "up"]);
});

test("successful source upload polls the returned deployment ID", (t) => {
  const result = upload(t, { output: JSON.stringify({ deploymentId: "cli-deploy" }), exitCode: 0 });
  assert.equal(result.status, 0, result.stderr);
  assert.deepEqual(result.events, ["service", "up", "deployment"]);
});

test("prepared CLI image activates and verifies without requiring backend transition settings", (t) => {
  const result = deploy(t, providerStatus([]), [], [], "cli-downloads");
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.events.filter((event) => event === "activate").length, 1);
  assert.equal(result.events.filter((event) => event === "verify").length, 1);
  assert.ok(!result.events.some((event) => event.startsWith("up ")));
});
