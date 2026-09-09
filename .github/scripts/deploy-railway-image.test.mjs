import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

const script = new URL("./deploy-railway-image.mjs", import.meta.url);
const digest = `sha256:${"a".repeat(64)}`;

function fixture(t) {
  const directory = mkdtempSync(join(tmpdir(), "scope-image-deploy-"));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  const railway = join(directory, "railway");
  const calls = join(directory, "calls.ndjson");
  writeFileSync(railway, `#!/usr/bin/env node
const { appendFileSync } = require("node:fs");
const args = process.argv.slice(2);
let input = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", chunk => input += chunk);
process.stdin.on("end", () => {
  appendFileSync(process.env.CALLS, JSON.stringify({ args, input, railwayToken: process.env.RAILWAY_TOKEN, apiToken: process.env.RAILWAY_API_TOKEN }) + "\\n");
  if (args[0] === "deployment") console.log(JSON.stringify([{ id: "deployment-123", serviceId: "service-123", status: "SUCCESS", meta: { imageDigest: process.env.DIGEST } }]));
  else if (args[1].includes("serviceInstanceDeployV2")) console.log(JSON.stringify({ data: { serviceInstanceDeployV2: "deployment-123" } }));
  else console.log(JSON.stringify({ data: { serviceInstanceUpdate: true } }));
});
`);
  chmodSync(railway, 0o755);
  return { directory, calls };
}

function baseEnv({ directory, calls }) {
  return {
    ...process.env,
    PATH: `${directory}:${process.env.PATH}`,
    CALLS: calls,
    DIGEST: digest,
    RAILWAY_API_TOKEN: "account-token",
    RAILWAY_TOKEN: "scoped-token",
    RAILWAY_PROJECT_ID: "project-123",
    SCOPE_RAILWAY_ENVIRONMENT_ID: "environment-123",
  };
}

test("activates and verifies the exact deployment returned by Railway", (t) => {
  const files = fixture(t);
  const evidence = join(files.directory, "evidence.ndjson");
  execFileSync(process.execPath, [script.pathname, "service-123", `ghcr.io/scope-vcs/scope-media-worker@${digest}`], {
    env: {
      ...baseEnv(files),
      SCOPE_DEPLOYMENT_COMPONENT: "media-worker",
      SCOPE_DEPLOYMENT_SOURCE_SHA: "b".repeat(40),
      SCOPE_DEPLOYMENT_EVIDENCE_PATH: evidence,
    },
  });

  const calls = readFileSync(files.calls, "utf8").trim().split("\n").map(JSON.parse);
  assert.equal(calls.length, 3);
  assert.match(calls[1].args[1], /serviceInstanceDeployV2/);
  assert.ok(calls.slice(0, 2).every(call => call.args.includes("@-") && !call.args.join(" ").includes(digest)));
  assert.ok(calls.every(call => call.apiToken === "account-token" && call.railwayToken === undefined));
  assert.deepEqual(JSON.parse(readFileSync(evidence, "utf8")), {
    component: "media-worker",
    sourceSha: "b".repeat(40),
    provider: "railway",
    evidenceId: "deployment-123",
    artifactDigest: digest,
  });
});

test("passes private registry credentials only through stdin", (t) => {
  const files = fixture(t);
  execFileSync(process.execPath, [script.pathname, "configure-registry", "service-123"], {
    env: {
      ...baseEnv(files),
      SCOPE_RAILWAY_REGISTRY_USERNAME: "registry-user",
      SCOPE_RAILWAY_REGISTRY_PASSWORD: "registry-secret",
    },
  });

  const [call] = readFileSync(files.calls, "utf8").trim().split("\n").map(JSON.parse);
  assert.equal(call.args.includes("@-"), true);
  assert.equal(call.args.join(" ").includes("registry-secret"), false);
  assert.deepEqual(JSON.parse(call.input).input.registryCredentials, {
    username: "registry-user",
    password: "registry-secret",
  });
});

test("rejects a successful deployment with a different image digest", (t) => {
  const files = fixture(t);
  assert.throws(() => execFileSync(process.execPath, [script.pathname, "service-123", `ghcr.io/scope-vcs/scope-media-worker@${digest}`], {
    env: {
      ...baseEnv(files),
      DIGEST: `sha256:${"c".repeat(64)}`,
      SCOPE_DEPLOYMENT_COMPONENT: "media-worker",
      SCOPE_DEPLOYMENT_SOURCE_SHA: "b".repeat(40),
      SCOPE_DEPLOYMENT_EVIDENCE_PATH: join(files.directory, "evidence.ndjson"),
    },
    stdio: "pipe",
  }), /did not match the reviewed digest/);
});
