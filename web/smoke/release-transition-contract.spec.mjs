import assert from "node:assert/strict";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  findReconnect,
  parseArguments,
  redactDiagnosticText,
  safeRequestUrl,
  verifyReleaseTransition,
} from "./release-transition.mjs";

test("release browser CLI parses explicit transition controls", () => {
  const options = parseArguments([
    "--base-url", "https://scope.example.test",
    "--repo", "dev/update-demo",
    "--activation-file", "/tmp/activation",
    "--ready-file", "/tmp/browser-ready",
    "--transition-file", "/tmp/old-teardown",
    "--summary", "/tmp/browser-summary",
    "--expected-activity-file", "/tmp/expected-activity",
    "--update-ready-file", "/tmp/update-ready",
    "--require-sse-reconnect", "false",
  ]);
  assert.equal(options.baseUrl, "https://scope.example.test");
  assert.equal(options.owner, "dev");
  assert.equal(options.repo, "update-demo");
  assert.equal(options.requireSseReconnect, false);
  assert.throws(
    () => parseArguments([
      "--base-url", "https://scope.example.test/path",
      "--repo", "dev/update-demo",
      "--activation-file", "/tmp/activation",
      "--ready-file", "/tmp/ready",
      "--transition-file", "/tmp/transition",
      "--summary", "/tmp/summary",
    ]),
    /without credentials or a path/,
  );
  assert.throws(
    () => parseArguments([
      "--base-url", "https://scope.example.test",
      "--repo", "dev/update-demo",
      "--activation-file", "/tmp/activation",
      "--ready-file", "/tmp/ready",
      "--transition-file", "/tmp/transition",
      "--summary", "/tmp/summary",
      "--expected-activity-file", "/tmp/activity",
    ]),
    /must be provided together/,
  );
});

test("reconnect evidence is paired after interruption and bounded", () => {
  const starts = [
    { at: "2026-09-06T12:00:00.000Z", sequence: 1 },
    { at: "2026-09-06T12:00:06.000Z", confirmedAt: "2026-09-06T12:00:06.000Z", sequence: 2 },
  ];
  const ends = [{ at: "2026-09-06T12:00:05.000Z", outcome: "failed", sequence: 1 }];
  assert.deepEqual(
    findReconnect(starts, ends, "2026-09-06T12:00:04.000Z", 10_000),
    {
      interruptedAt: "2026-09-06T12:00:05.000Z",
      interruptionOutcome: "failed",
      interruptionCount: 1,
      reconnectedAt: "2026-09-06T12:00:06.000Z",
      reconnectMs: 1_000,
    },
  );
  assert.equal(
    findReconnect(
      [starts[0], { at: "2026-09-06T12:00:06.000Z", confirmedAt: "2026-09-06T12:00:16.000Z", sequence: 2 }],
      ends,
      "2026-09-06T12:00:04.000Z",
      10_000,
    ),
    null,
  );
  assert.equal(
    findReconnect(starts, [
      ...ends,
      { at: "2026-09-06T12:00:07.000Z", outcome: "failed", sequence: 2 },
    ], "2026-09-06T12:00:04.000Z", 10_000),
    null,
  );
});

test("browser diagnostics omit request secrets and bound console text", () => {
  assert.equal(
    safeRequestUrl("https://scope.example.test/_server?token=secret#fragment"),
    "https://scope.example.test/_server",
  );
  const diagnostic = redactDiagnosticText(
    `authorization: Bearer secret https://scope.example.test/_server?token=secret ${"x".repeat(400)}`,
  );
  assert.equal(diagnostic.includes("secret"), false);
  assert.equal(diagnostic.includes("?"), false);
  assert.equal(diagnostic.length, 300);
});

test("browser refuses a stale teardown signal before opening", async (t) => {
  const root = await mkdtemp(join(tmpdir(), "scope-browser-transition-"));
  t.after(() => rm(root, { force: true, recursive: true }));
  const transitionFile = join(root, "teardown");
  await writeFile(transitionFile, "stale");
  await assert.rejects(
    verifyReleaseTransition({
      activityTimeoutMs: 1_000,
      activationFile: join(root, "activation"),
      baseUrl: "https://scope.example.test",
      expectedActivityFile: undefined,
      owner: "dev",
      readyFile: join(root, "ready"),
      reconnectBoundMs: 1_000,
      repo: "update-demo",
      requireSseReconnect: true,
      summaryFile: join(root, "summary"),
      transitionFile,
      transitionTimeoutMs: 1_000,
      updateReadyFile: undefined,
    }),
    /must open before activation/,
  );
});

 test("reconnect attempts without received SSE frames do not pass", () => {
  const ends = [{ at: "2026-09-06T12:00:05.000Z", outcome: "failed", sequence: 1 }];
  for (const evidence of [{}, {status: 503}, {status: 200}, {confirmedAt: "2026-09-06T12:00:06.000Z", observationFailed: true}]) {
    assert.equal(findReconnect([{at: "2026-09-06T12:00:06.000Z", sequence: 2, ...evidence}], ends, "2026-09-06T12:00:04.000Z", 10000), null);
  }
});
