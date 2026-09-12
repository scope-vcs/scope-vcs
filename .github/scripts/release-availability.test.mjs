import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";
import {
  AvailabilityEvidence,
  availabilityTargets,
  parseAvailabilityConfig,
  probeTarget,
} from "./release-availability-core.mjs";
import { runAvailabilityProbe } from "./release-availability.mjs";

const SCRIPT = join(dirname(fileURLToPath(import.meta.url)), "release-availability.mjs");
const WRAPPER = join(dirname(fileURLToPath(import.meta.url)), "with-release-availability.sh");
const SOURCE_SHA = "a".repeat(40);

test("constructs and verifies the real public release requests", async (t) => {
  const requests = [];
  let repositoryCalls = 0;
  const web = await listen((request, response) => {
    requests.push(request.url);
    if (request.url === "/") return html(response, homepage());
    if (request.url === "/readyz") return json(response, readiness("web"));
    response.writeHead(404).end();
  });
  const api = await listen((request, response) => {
    requests.push(request.url);
    if (request.url === "/readyz") return json(response, readiness("api", ["database", "object_store"]));
    if (request.url === "/v1/repos/dev/public-demo") {
      repositoryCalls += 1;
      if (repositoryCalls === 1) return json(response, { message: "switching" }, 503);
      return json(response, {
        id: "repo_fixture",
        lifecycle_state: "Ready",
        name: "public-demo",
        owner_handle: "dev",
      });
    }
    if (request.url === "/v1/repos/dev/public-demo/files/content?path=README.html") {
      return json(response, {
        content: { kind: "text", text: "Public by design." },
        path: "/README.html",
      });
    }
    if (request.url === "/v1/repos/dev/public-demo/requests") {
      return json(response, { next_cursor: null, requests: [{ id: "req_demo_ready" }] });
    }
    response.writeHead(404).end();
  });
  t.after(async () => {
    await Promise.all([close(web.server), close(api.server)]);
  });

  const root = await mkdtemp(join(tmpdir(), "scope-availability-"));
  t.after(() => rm(root, { force: true, recursive: true }));
  const paths = {
    config: join(root, "config.json"),
    deployments: join(root, "deployments.json"),
    events: join(root, "events.ndjson"),
    ready: join(root, "ready.json"),
    stop: join(root, "stop"),
    summary: join(root, "summary.json"),
  };
  await writeFile(paths.deployments, JSON.stringify({ api: "old-api", web: "old-web" }));
  await writeFile(paths.config, JSON.stringify(config(web.origin, api.origin, {
    intervalMs: 1_000,
    release: {
      attemptId: "release-one",
      deploymentsFile: paths.deployments,
      sourceSha: SOURCE_SHA,
      stage: "ordinary-handoff",
    },
  })));
  const child = spawn(process.execPath, [
    SCRIPT,
    "--config", paths.config,
    "--events", paths.events,
    "--summary", paths.summary,
    "--ready-file", paths.ready,
    "--stop-file", paths.stop,
  ], { stdio: ["ignore", "pipe", "pipe"] });
  const output = collect(child.stdout);
  const errors = collect(child.stderr);
  await waitForFile(paths.ready);
  await writeFile(paths.deployments, JSON.stringify({ api: "new-api", web: "new-web" }));
  await new Promise((resolve) => setTimeout(resolve, 1_100));
  await writeFile(paths.stop, "stop\n");
  const [status] = await once(child, "exit");
  assert.equal(status, 1, await errors);

  const summary = JSON.parse(await readFile(paths.summary, "utf8"));
  const events = (await readFile(paths.events, "utf8")).trim().split("\n").map(JSON.parse);
  assert.equal(summary.passed, false);
  assert.equal(summary.failureCount, 1);
  assert.deepEqual(summary.deploymentSnapshots.map(({ deployments }) => deployments), [
    { api: "old-api", web: "old-web" },
    { api: "new-api", web: "new-web" },
  ]);
  assert.match(summary.violations[0], /ordinary release/);
  assert.equal(events.find(({ ok }) => !ok).target, "fixture-repository");
  assert.deepEqual(events.find(({ ok }) => !ok).deployments, {
    api: "old-api",
    web: "old-web",
  });
  assert.equal(events.at(-1).ok, true);
  assert(summary.sampleCount >= 2);
  assert.deepEqual(JSON.parse(await output), summary);
  for (const path of [
    "/",
    "/readyz",
    "/v1/repos/dev/public-demo",
    "/v1/repos/dev/public-demo/files/content?path=README.html",
    "/v1/repos/dev/public-demo/requests",
  ]) {
    assert(requests.includes(path), `missing request ${path}`);
  }
});

test("reads the current homepage without a browser handshake and still rejects redirects", async (t) => {
  let redirectPage = false;
  const web = await listen((request, response) => {
    if (request.headers.accept?.includes("text/html") || redirectPage) {
      response.writeHead(307, { location: "/handshake" }).end();
      return;
    }
    html(response, homepage());
  });
  t.after(() => close(web.server));
  const target = availabilityTargets(parseAvailabilityConfig(config(web.origin, web.origin)))[0];
  assert.equal((await probeTarget(target, { timeoutMs: 1_000 })).ok, true);
  redirectPage = true;
  assert.equal((await probeTarget(target, { timeoutMs: 1_000 })).ok, false);
});

test("rejects application errors returned with HTTP 200", async () => {
  const parsed = parseAvailabilityConfig(config("https://web.example.test", "https://api.example.test"));
  const requestList = availabilityTargets(parsed).find(({ name }) => name === "fixture-request-list");
  const result = await probeTarget(requestList, {
    fetchImpl: async () => new Response(JSON.stringify({
      error: { code: "projection_unavailable", message: "not ready" },
      status: "ok",
    }), { headers: { "content-type": "application/json" }, status: 200 }),
    timeoutMs: 1_000,
  });
  assert.equal(result.ok, false);
  assert.equal(result.error.kind, "application");

  const homepage = availabilityTargets(parsed).find(({ name }) => name === "public-homepage");
  const htmlResult = await probeTarget(homepage, {
    fetchImpl: async () => new Response("<!doctype html><title>Scope</title>Internal Server Error", {
      headers: { "content-type": "text/html" },
    }),
    timeoutMs: 1_000,
  });
  assert.equal(htmlResult.ok, false);
  assert.equal(htmlResult.error.kind, "application");
});

test("wrapper runs activation under monitoring and retains the tail summary", async (t) => {
  let failRepository = false;
  const web = await listen((request, response) => {
    if (request.url === "/") return html(response, homepage());
    if (request.url === "/readyz") return json(response, readiness("web"));
    response.writeHead(404).end();
  });
  const api = await listen((request, response) => {
    if (request.url === "/readyz") return json(response, readiness("api"));
    if (request.url === "/v1/repos/dev/public-demo") {
      if (failRepository) {
        failRepository = false;
        return json(response, { message: "writers closed" }, 503);
      }
      return json(response, {
        id: "repo_fixture",
        lifecycle_state: "Ready",
        name: "public-demo",
        owner_handle: "dev",
      });
    }
    if (request.url === "/v1/repos/dev/public-demo/files/content?path=README.html") {
      return json(response, {
        content: { kind: "text", text: "Public by design." },
        path: "/README.html",
      });
    }
    if (request.url === "/v1/repos/dev/public-demo/requests") {
      return json(response, { next_cursor: null, requests: [] });
    }
    response.writeHead(404).end();
  });
  t.after(async () => Promise.all([close(web.server), close(api.server)]));
  const root = await mkdtemp(join(tmpdir(), "scope-availability-wrapper-"));
  t.after(() => rm(root, { force: true, recursive: true }));
  const configPath = join(root, "config.json");
  const outputPath = join(root, "evidence");
  const activationPath = join(root, "activated");
  await writeFile(configPath, JSON.stringify(config(web.origin, api.origin, {
    fixture: {
      expectedRequestIds: [],
      expectedText: "Public by design.",
      filePath: "README.html",
      owner: "dev",
      repo: "public-demo",
    },
    intervalMs: 1_000,
  })));
  const child = spawn("bash", [
    WRAPPER,
    configPath,
    outputPath,
    "--",
    process.execPath,
    "-e",
    "require('node:fs').writeFileSync(process.argv[1], 'activated')",
    activationPath,
  ], {
    env: {
      ...process.env,
      SCOPE_RELEASE_OBSERVATION_SECONDS: "0",
      SCOPE_RELEASE_READY_TIMEOUT_SECONDS: "5",
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  const output = collect(child.stdout);
  const errors = collect(child.stderr);
  const [status] = await once(child, "exit");
  assert.equal(status, 0, `${await output}\n${await errors}`);
  assert.equal(await readFile(activationPath, "utf8"), "activated");
  const summary = JSON.parse(await readFile(join(outputPath, "availability-summary.json"), "utf8"));
  assert.equal(summary.passed, true);
  assert(summary.sampleCount >= 1);
  assert.equal(summary.failureCount, 0);

  failRepository = true;
  const recoveryConfigPath = join(root, "recovery-config.json");
  const recoveryOutputPath = join(root, "recovery-evidence");
  const recoveryActivationPath = join(root, "recovered");
  const maintenanceStart = join(root, "maintenance-start");
  const maintenanceEnd = join(root, "maintenance-end");
  await writeFile(maintenanceStart, String(Date.now() - 500));
  await writeFile(recoveryConfigPath, JSON.stringify(config(web.origin, api.origin, {
    fixture: {
      expectedRequestIds: [],
      expectedText: "Public by design.",
      filePath: "README.html",
      owner: "dev",
      repo: "public-demo",
    },
    intervalMs: 1_000,
    maintenance: {
      endFile: maintenanceEnd,
      warningAfterMs: 10_000,
      startFile: maintenanceStart,
    },
    mode: "maintenance",
  })));
  const recovery = spawn("bash", [
    WRAPPER,
    recoveryConfigPath,
    recoveryOutputPath,
    "--",
    process.execPath,
    "-e",
    "const fs=require('node:fs');fs.writeFileSync(process.argv[1], 'recovered');fs.writeFileSync(process.argv[2], String(Date.now()))",
    recoveryActivationPath,
    maintenanceEnd,
  ], {
    env: {
      ...process.env,
      SCOPE_RELEASE_OBSERVATION_SECONDS: "0",
      SCOPE_RELEASE_READY_TIMEOUT_SECONDS: "5",
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  const recoveryOutput = collect(recovery.stdout);
  const recoveryErrors = collect(recovery.stderr);
  const [recoveryStatus] = await once(recovery, "exit");
  assert.equal(recoveryStatus, 0, `${await recoveryOutput}\n${await recoveryErrors}`);
  assert.equal(await readFile(recoveryActivationPath, "utf8"), "recovered");
  const recoverySummary = JSON.parse(
    await readFile(join(recoveryOutputPath, "availability-summary.json"), "utf8"),
  );
  assert.equal(recoverySummary.passed, true);
  assert.equal(recoverySummary.outsideMaintenanceFailureCount, 0);
  assert(recoverySummary.maintenanceFailureCount >= 1);
  assert(recoverySummary.maintenanceWindow.endedAt);
});

test("accounts for an explicit maintenance window and warns without failing a restored release", () => {
  const parsed = parseAvailabilityConfig(config(
    "https://web.example.test",
    "https://api.example.test",
    {
      maintenance: {
        endFile: "/tmp/scope-maintenance-end",
        warningAfterMs: 1_000,

        startFile: "/tmp/scope-maintenance-start",
      },
      mode: "maintenance",
    },
  ));
  const evidence = new AvailabilityEvidence(parsed, "2026-09-06T12:00:00.000Z");
  assert.equal(evidence.finish("2026-09-06T12:00:00.500Z").passed, true);
  evidence.openMaintenance("2026-09-06T12:00:01.000Z");
  evidence.record(failedEvent("2026-09-06T12:00:01.100Z", "maintenance"));
  evidence.record(failedEvent("2026-09-06T12:00:01.200Z", "maintenance"));
  evidence.closeMaintenance("2026-09-06T12:00:01.800Z");
  const passing = evidence.finish("2026-09-06T12:00:02.000Z");
  assert.equal(passing.passed, true);
  assert.equal(passing.maintenanceWindow.durationMs, 800);
  assert.equal(passing.maintenanceFailureCount, 2);

  const overrun = new AvailabilityEvidence(parsed, "2026-09-06T12:00:00.000Z");
  overrun.openMaintenance("2026-09-06T12:00:00.000Z");
  for (let index = 0; index < 10; index += 1) {
    overrun.record(failedEvent("2026-09-06T12:00:01.100Z", "maintenance"));
  }
  overrun.closeMaintenance("2026-09-06T12:00:02.000Z");
  const restored = overrun.finish("2026-09-06T12:00:03.000Z");
  assert.equal(restored.passed, true);
  assert.match(restored.warnings[0], /exceeded warning threshold/);

  evidence.record(failedEvent("2026-09-06T12:00:01.900Z", "serving"));
  const failed = evidence.finish("2026-09-06T12:00:02.100Z");
  assert.equal(failed.passed, false);
  assert.match(failed.violations[0], /outside maintenance/);
});

test("classifies finite requests at completion across maintenance boundaries", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "scope-availability-boundary-"));
  t.after(() => rm(root, { force: true, recursive: true }));

  const during = await runBoundaryCase(root, "during", async ({ fetchStarted, startFile }) => {
    await fetchStarted;
    await writeFile(startFile, String(Date.now()));
  });
  const duringFailure = during.failures.find(({ target }) => target === "fixture-repository");
  assert(duringFailure);
  assert.equal(duringFailure.phase, "maintenance");
  assert(Date.parse(duringFailure.startedAt) < Date.parse(duringFailure.at));
  assert.equal(during.passed, true);

  const after = await runBoundaryCase(root, "after", async ({ endFile, fetchStarted }) => {
    await fetchStarted;
    await writeFile(endFile, String(Date.now()));
  }, { startBeforeRequest: true });
  const afterFailure = after.failures.find(({ target }) => target === "fixture-repository");
  assert(afterFailure);
  assert.equal(afterFailure.phase, "serving");
  assert(Date.parse(afterFailure.startedAt) < Date.parse(afterFailure.at));
  assert.equal(after.passed, false);
  assert.match(after.violations.join("\n"), /outside maintenance/);
});

test("orders overlapping failures and deployment snapshots by completion time", () => {
  const parsed = parseAvailabilityConfig(config(
    "https://web.example.test",
    "https://api.example.test",
  ));
  const evidence = new AvailabilityEvidence(parsed, "2026-09-06T12:00:00.000Z");
  evidence.record({
    ...failedEvent("2026-09-06T12:00:02.000Z", "serving"),
    deployments: { api: "new-api" },
  });
  evidence.record({
    ...failedEvent("2026-09-06T12:00:01.000Z", "serving"),
    deployments: { api: "old-api" },
  });
  const summary = evidence.finish("2026-09-06T12:00:03.000Z");
  assert.equal(summary.firstFailureAt, "2026-09-06T12:00:01.000Z");
  assert.equal(summary.lastFailureAt, "2026-09-06T12:00:02.000Z");
  assert.deepEqual(summary.failures.map(({ at }) => at), [
    "2026-09-06T12:00:01.000Z",
    "2026-09-06T12:00:02.000Z",
  ]);
  assert.deepEqual(summary.deploymentSnapshots.map(({ deployments }) => deployments), [
    { api: "old-api" },
    { api: "new-api" },
  ]);
});

test("rejects ambiguous origins and maintenance configuration on ordinary releases", () => {
  assert.throws(
    () => parseAvailabilityConfig(config("https://token@example.test", "https://api.example.test")),
    /without credentials/,
  );
  assert.throws(
    () => parseAvailabilityConfig(config("https://web.example.test/path", "https://api.example.test")),
    /without credentials or a path/,
  );
  assert.throws(
    () => parseAvailabilityConfig(config(
      "https://web.example.test",
      "https://api.example.test",
      { maintenance: {} },
    )),
    /ordinary releases cannot define/,
  );
});

function config(webOrigin, apiOrigin, overrides = {}) {
  return {
    apiOrigin,
    fixture: {
      expectedText: "Public by design.",
      expectedRequestIds: ["req_demo_ready"],
      filePath: "README.html",
      owner: "dev",
      repo: "public-demo",
    },
    intervalMs: 1_000,
    mode: "ordinary",
    release: {
      attemptId: "release-one",
      sourceSha: SOURCE_SHA,
      stage: "ordinary-handoff",
    },
    requestTimeoutMs: 1_000,
    webOrigin,
    ...overrides,
  };
}

function failedEvent(at, phase) {
  return {
    at,
    error: { kind: "http-status", message: "HTTP 503" },
    ok: false,
    phase,
    release: { attemptId: "release-one", sourceSha: SOURCE_SHA, stage: "cutover" },
    sample: 1,
    target: "api-readiness",
  };
}

async function runBoundaryCase(root, name, moveBoundary, { startBeforeRequest = false } = {}) {
  const directory = join(root, name);
  const paths = {
    config: join(directory, "config.json"),
    events: join(directory, "events.ndjson"),
    ready: join(directory, "ready.json"),
    stop: join(directory, "stop"),
    summary: join(directory, "summary.json"),
  };
  const startFile = join(directory, "maintenance-start");
  const endFile = join(directory, "maintenance-end");
  await mkdir(directory);
  if (startBeforeRequest) await writeFile(startFile, String(Date.now() - 10));
  await writeFile(paths.config, JSON.stringify(config(
    "https://web.example.test",
    "https://api.example.test",
    {
      maintenance: {
        endFile,
        warningAfterMs: 10_000,

        startFile,
      },
      mode: "maintenance",
    },
  )));
  let markFetchStarted;
  const fetchStarted = new Promise((resolve) => { markFetchStarted = resolve; });
  const running = runAvailabilityProbe(paths, {
    fetchImpl: async (url) => {
      markFetchStarted();
      await new Promise((resolve) => setTimeout(resolve, 80));
      return boundaryResponse(url);
    },
  });
  await moveBoundary({ endFile, fetchStarted, startFile });
  await waitForFile(paths.ready);
  if (!startBeforeRequest) await writeFile(endFile, String(Date.now()));
  await writeFile(paths.stop, "stop\n");
  return running;
}

function boundaryResponse(value) {
  const url = new URL(value);
  if (url.hostname === "web.example.test" && url.pathname === "/") {
    return new Response(homepage(), { headers: { "content-type": "text/html" } });
  }
  if (url.pathname === "/readyz") {
    return Response.json(readiness(url.hostname.startsWith("web.") ? "web" : "api"));
  }
  if (url.pathname === "/v1/repos/dev/public-demo") {
    return Response.json({ message: "transition boundary" }, { status: 503 });
  }
  if (url.pathname.endsWith("/files/content")) {
    return Response.json({
      content: { kind: "text", text: "Public by design." },
      path: "/README.html",
    });
  }
  if (url.pathname.endsWith("/requests")) {
    return Response.json({ next_cursor: null, requests: [{ id: "req_demo_ready" }] });
  }
  throw new Error(`unexpected URL ${url.origin}${url.pathname}`);
}

function readiness(service, checks = []) {
  return {
    checks: checks.map((name) => ({ name, status: "ok" })),
    service,
    status: "ok",
  };
}

function homepage() {
  return '<!doctype html><title>Scope</title><main data-scope-page="landing"><h1>Landing</h1></main>';
}

function html(response, body) {
  response.writeHead(200, { "content-type": "text/html" });
  response.end(body);
}

function json(response, body, status = 200) {
  response.writeHead(status, { "content-type": "application/json" });
  response.end(JSON.stringify(body));
}

async function listen(handler) {
  const server = createServer(handler);
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  return { origin: `http://127.0.0.1:${address.port}`, server };
}

async function close(server) {
  server.close();
  await once(server, "close");
}

async function waitForFile(path) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    try {
      await readFile(path);
      return;
    } catch (error) {
      if (error?.code !== "ENOENT") throw error;
    }
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
  throw new Error(`timed out waiting for ${path}`);
}

async function collect(stream) {
  let value = "";
  for await (const chunk of stream) value += chunk;
  return value.trim();
}


test("regular release errors are informational until recovery observation, which must sample all targets", () => {
  const parsed = parseAvailabilityConfig(config("https://web.example.test", "https://api.example.test"));
  parsed.release.observationStartFile = "/tmp/observation-start";
  const evidence = new AvailabilityEvidence(parsed, "2026-09-06T12:00:00.000Z");
  evidence.record(failedEvent("2026-09-06T12:00:01.000Z", "serving"));
  assert.equal(evidence.finish().passed, false);
  evidence.observationStartedAt = "2026-09-06T12:00:02.000Z";
  assert.equal(evidence.finish().passed, false);
  for (const { name } of availabilityTargets(parsed)) {
    evidence.record({ at: "2026-09-06T12:00:03.000Z", target: name, phase: "serving", ok: true });
  }
  evidence.record({
    ...failedEvent("2026-09-06T12:00:03.500Z", "serving"),
    startedAt: "2026-09-06T12:00:01.900Z",
  });
  assert.equal(evidence.finish().passed, true);
  evidence.record(failedEvent("2026-09-06T12:00:04.000Z", "serving"));
  assert.equal(evidence.finish().passed, false);
});
