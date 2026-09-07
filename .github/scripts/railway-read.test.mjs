import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

import { readRailway } from "./railway-read.mjs";

test("discards failed stdout and invalid JSON before returning the successful read", () => {
  let attempts = 0;
  const delays = [];
  const reports = [];
  const result = readRailway(["status", "--json"], {
    execute(command, args, options) {
      assert.equal(command, "railway");
      assert.deepEqual(args, ["status", "--json"]);
      assert.deepEqual(options, {
        input: undefined,
        encoding: "utf8",
        stdio: ["pipe", "pipe", "pipe"],
        timeout: 30_000,
        killSignal: "SIGKILL",
      });
      attempts += 1;
      if (attempts === 1) throw Object.assign(new Error("secret error"), { stdout: '{"secret":"partial' });
      if (attempts === 2) return '{"secret":"invalid';
      return '{"id":"project"}';
    },
    pause: (delay) => delays.push(delay),
    report: (message) => reports.push(message),
  });
  assert.deepEqual(result, { id: "project" });
  assert.equal(attempts, 3);
  assert.deepEqual(delays, [2_000, 2_000]);
  assert.deepEqual(reports, [
    "Railway read failed; retrying (1/3)",
    "Railway read failed; retrying (2/3)",
  ]);
});

test("bounds failures without exposing variable values or command errors", () => {
  let attempts = 0;
  const delays = [];
  const reports = [];
  assert.throws(() => readRailway(["variable", "list", "--json"], {
    execute() {
      attempts += 1;
      throw Object.assign(new Error("SECRET raw command failure"), {
        stdout: '{"TOKEN":"SECRET"}', stderr: "SECRET diagnostics",
      });
    },
    pause: (delay) => delays.push(delay),
    report: (message) => reports.push(message),
  }), { message: "Railway read failed after 3 attempts" });
  assert.equal(attempts, 3);
  assert.deepEqual(delays, [2_000, 2_000]);
  assert.equal(reports.length, 2);
  assert.doesNotMatch(reports.join("\n"), /SECRET/);
});

test("passes GraphQL variables through stdin on every attempt", () => {
  const query = "query Project($id: String!) { project(id: $id) { id } }";
  const input = JSON.stringify({ id: "SECRET" });
  let attempts = 0;
  assert.deepEqual(readRailway(["api", query, "--variables", "@-"], {
    input,
    execute(_command, args, options) {
      attempts += 1;
      assert.equal(options.input, input);
      assert.doesNotMatch(JSON.stringify(args), /SECRET/);
      return attempts === 1
        ? '{"data":null,"errors":[{"message":"SECRET"}]}'
        : '{"data":{"project":{"id":"project"}},"errors":[]}';
    },
    pause: () => {},
    report: (message) => assert.doesNotMatch(message, /SECRET/),
  }), { data: { project: { id: "project" } }, errors: [] });
  assert.equal(attempts, 2);
});

test("bounds GraphQL error responses even when they contain partial data", () => {
  let attempts = 0;
  assert.throws(() => readRailway(["api", "query { me { id } }"], {
    execute() {
      attempts += 1;
      return '{"data":{"me":{"id":"partial"}},"errors":[{"message":"SECRET"}]}';
    },
    pause: () => {},
    report: () => {},
  }), { message: "Railway read failed after 3 attempts" });
  assert.equal(attempts, 3);
});

test("rejects mutations, unapproved commands, and multiple operations before execution", () => {
  for (const args of [
    ["up"], ["restart"], ["service", "delete"], ["variable", "set", "TOKEN=SECRET"],
    ["api", "mutation { serviceDelete(id: \"service\") }"],
    ["api", "subscription { events }"],
    ["api", "{ me { id } }"],
    ["api", "query One { me { id } } query Two { me { id } }"],
    ["api", "query One { me { id } } { me { id } }"],
    ["api", "query One { me { id } } mutation Delete { serviceDelete(id: \"service\") }"],
    ["api", "query { me { id } }", "--variables", '{"id":"SECRET"}'],
    ["api", "query { me { id } }", "--file", "mutation.graphql"],
    ["api", "query { me { id }"],
  ]) {
    assert.throws(() => readRailway(args, {
      execute: () => assert.fail("unsupported command was executed"),
      pause: () => assert.fail("unsupported command was retried"),
    }), { message: "Unsupported Railway read command" });
  }
});

test("allows each approved read and nested queries with quoted braces", () => {
  for (const args of [
    ["status", "--json"], ["service", "list", "--json"],
    ["deployment", "list", "--service", "api", "--json"], ["variable", "list", "--json"],
    ["api", 'query Example($id: String = "}") { project(id: $id) { id } }'],
  ]) {
    assert.deepEqual(readRailway(args, {
      execute: () => '{"ok":true}',
      pause: () => assert.fail("successful read was retried"),
    }), { ok: true });
  }
});

test("CLI outputs only JSON and does not wait for stdin on routine reads", async (context) => {
  const directory = mkdtempSync(join(tmpdir(), "railway-read-"));
  context.after(() => rmSync(directory, { recursive: true, force: true }));
  writeFileSync(join(directory, "railway"), '#!/usr/bin/env node\nprocess.stdout.write(JSON.stringify({ ok: true }));\n', { mode: 0o755 });
  const script = fileURLToPath(new URL("./railway-read.mjs", import.meta.url));
  const child = spawn(process.execPath, [script, "status", "--json"], {
    env: { ...process.env, PATH: `${directory}:${process.env.PATH}` },
    stdio: ["pipe", "pipe", "pipe"],
  });
  context.after(() => child.kill("SIGKILL"));
  let stdout = "";
  let stderr = "";
  child.stdout.on("data", (chunk) => { stdout += chunk; });
  child.stderr.on("data", (chunk) => { stderr += chunk; });
  const code = await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("CLI waited for open stdin")), 5_000);
    child.on("error", reject);
    child.on("close", (exitCode) => { clearTimeout(timer); resolve(exitCode); });
  });
  assert.equal(code, 0);
  assert.equal(stdout, '{"ok":true}\n');
  assert.equal(stderr, "");

  writeFileSync(join(directory, "railway"), '#!/usr/bin/env node\nconst fs = require("node:fs");\nprocess.stdout.write(JSON.stringify({ received: JSON.parse(fs.readFileSync(0, "utf8")).id }));\n', { mode: 0o755 });
  assert.equal(execFileSync(process.execPath, [script, "api", "query { me { id } }", "--variables", "@-"], {
    input: '{"id":"stdin-value"}', encoding: "utf8",
    env: { ...process.env, PATH: `${directory}:${process.env.PATH}` },
    timeout: 5_000,
  }), '{"received":"stdin-value"}\n');
});
