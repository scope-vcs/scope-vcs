import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdtemp, mkdir, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import test from "node:test";
import ts from "typescript";

import { analyzeSnapshot } from "../src/analyzer.mjs";
import { ANALYZER_VERSION } from "../src/constants.mjs";

const execute = promisify(execFile);
const here = dirname(fileURLToPath(import.meta.url));
const fixture = (name) => resolve(here, "fixtures", name);

function edgesFrom(result, source) {
  return result.edges.filter((edge) => edge.source_path === source);
}

test("retains static import forms and deduplicates relative and aliased targets", async () => {
  const result = await analyzeSnapshot(fixture("static"));

  assert.equal(result.analyzer_version, ANALYZER_VERSION);
  assert.deepEqual(result.gaps, []);
  assert.deepEqual(
    result.edges.map(({ source_path, kind }) => [source_path, kind]),
    [
      ["src/public/alias.ts", "import"],
      ["src/public/direct.ts", "import"],
      ["src/public/duplicate.ts", "import"],
      ["src/public/literal-dynamic.ts", "dynamic-import"],
      ["src/public/namespace.ts", "import"],
      ["src/public/reexport.ts", "re-export"],
      ["src/public/require.cjs", "require"],
      ["src/public/side-effect.ts", "side-effect-import"],
      ["src/public/type.ts", "type-import"],
    ],
  );
  assert.deepEqual(edgesFrom(result, "src/public/external.ts"), []);
  assert.deepEqual(edgesFrom(result, "src/public/duplicate.ts"), [
    {
      source_path: "src/public/duplicate.ts",
      target_path: "src/private/pricing.ts",
      kind: "import",
    },
  ]);
});

test("keeps known edges while reporting unresolved, nonliteral, and syntax gaps", async () => {
  const result = await analyzeSnapshot(fixture("incomplete"));

  assert.deepEqual(result.edges, [
    {
      source_path: "src/public/known.ts",
      target_path: "src/private/value.ts",
      kind: "import",
    },
  ]);
  assert.ok(result.gaps.some(({ path, reason }) => path === "src/public/missing.ts" && reason.startsWith("unresolved import:")));
  assert.ok(result.gaps.some(({ path, reason }) => path === "src/public/runtime.ts" && reason === "non-literal dynamic import"));
  assert.ok(result.gaps.some(({ path, reason }) => path === "src/public/runtime.ts" && reason === "non-literal require"));
  assert.ok(result.gaps.some(({ path, reason }) => path === "src/public/syntax-error.ts" && reason.startsWith("syntax error:")));
});

test("reports unsupported source languages and retains internal data-file edges", async () => {
  const result = await analyzeSnapshot(fixture("mixed"));

  assert.deepEqual(result.unsupported_files, ["src/main.rs", "src/private.py"]);
  assert.deepEqual(result.edges, [
    {
      source_path: "src/config.ts",
      target_path: "data.json",
      kind: "import",
    },
  ]);
  assert.deepEqual(result.gaps, []);
  assert.ok(!result.analyzed_files.includes("README.md"));
});

test("resolves aliases inherited by a nested monorepo tsconfig", async () => {
  const result = await analyzeSnapshot(fixture("monorepo"));
  assert.deepEqual(result.gaps, []);
  assert.deepEqual(result.edges, [
    {
      source_path: "packages/app/src/main.ts",
      target_path: "packages/shared/src/value.ts",
      kind: "import",
    },
  ]);
});

test("resolves child path aliases against an inherited baseUrl", async () => {
  const result = await analyzeSnapshot(fixture("inherited-base-url"));
  assert.deepEqual(result.gaps, []);
  assert.deepEqual(result.edges, [
    {
      source_path: "packages/app/src/main.ts",
      target_path: "packages/shared/src/value.ts",
      kind: "import",
    },
  ]);
});

test("reports invalid TypeScript config without losing relative edges", async () => {
  const result = await analyzeSnapshot(fixture("bad-config"));
  assert.deepEqual(result.edges, [
    {
      source_path: "src/entry.ts",
      target_path: "src/value.ts",
      kind: "import",
    },
  ]);
  assert.ok(result.gaps.some(({ path, reason }) =>
    path === "tsconfig.json" && reason.startsWith("invalid TypeScript config:"),
  ));
});

test("does not execute repository analyzer configuration", async () => {
  const result = await analyzeSnapshot(fixture("no-execution"));
  assert.deepEqual(result.gaps, []);
  assert.deepEqual(edgesFrom(result, "src/entry.js"), [
    {
      source_path: "src/entry.js",
      target_path: "src/value.js",
      kind: "import",
    },
  ]);
});

test("refuses symbolic links instead of following snapshot escapes", async (context) => {
  const root = await mkdtemp(resolve(tmpdir(), "scope-dependency-analyzer-"));
  context.after(() => rm(root, { recursive: true }));
  await mkdir(resolve(root, "src"));
  await writeFile(resolve(root, "src", "main.ts"), "export const safe = true;\n");
  await symlink(resolve(tmpdir(), "outside.ts"), resolve(root, "src", "escape.ts"));

  const result = await analyzeSnapshot(root);
  assert.deepEqual(result.analyzed_files, []);
  assert.deepEqual(result.edges, []);
  assert.deepEqual(result.gaps, [
    { path: ".", reason: "symbolic links are not analyzed: src/escape.ts" },
  ]);
});

test("reports imports resolving outside the snapshot without retaining an edge", async (context) => {
  const parent = await mkdtemp(resolve(tmpdir(), "scope-dependency-escape-"));
  context.after(() => rm(parent, { recursive: true }));
  const root = resolve(parent, "repo");
  await mkdir(resolve(root, "src"), { recursive: true });
  await writeFile(resolve(parent, "outside.ts"), "export const outside = true;\n");
  await writeFile(
    resolve(root, "src", "main.ts"),
    "import { outside } from '../../outside';\nexport const escaped = outside;\n",
  );

  const result = await analyzeSnapshot(root);
  assert.deepEqual(result.edges, []);
  assert.deepEqual(result.gaps, [
    { path: "src/main.ts", reason: "resolved target escapes snapshot: ../../outside" },
  ]);
});

test("unresolved bare imports are external unless the repository claims the name", async (context) => {
  const root = await mkdtemp(resolve(tmpdir(), "scope-dependency-external-"));
  context.after(() => rm(root, { recursive: true }));
  await mkdir(resolve(root, "packages/shared"), { recursive: true });
  await mkdir(resolve(root, "src/private"), { recursive: true });
  await writeFile(resolve(root, "package.json"), JSON.stringify({ name: "@acme/app", private: true }));
  await writeFile(resolve(root, "packages/shared/package.json"), JSON.stringify({ name: "@acme/shared" }));
  await writeFile(resolve(root, "tsconfig.json"), JSON.stringify({
    compilerOptions: { baseUrl: ".", paths: { "@alias/*": ["src/*"] } },
  }));
  await writeFile(resolve(root, "src/private/secret.ts"), "export const secret = 1;\n");
  await writeFile(resolve(root, "src/main.ts"), [
    "import React from 'react';",
    "import { z } from 'zod/v4';",
    "import path from 'node:path';",
    "import { shared } from '@acme/shared';",
    "import { missing } from '@alias/missing';",
    "import { mapped } from '#internal/mapped';",
    "import { secret } from './private/secret';",
    "export const value = [React, z, path, shared, missing, mapped, secret];",
    "",
  ].join("\n"));

  const result = await analyzeSnapshot(root);
  assert.deepEqual(result.edges, [
    { source_path: "src/main.ts", target_path: "src/private/secret.ts", kind: "import" },
  ]);
  assert.deepEqual(result.gaps.map(({ reason }) => reason).sort(), [
    "unresolved import: #internal/mapped",
    "unresolved import: @acme/shared",
    "unresolved import: @alias/missing",
  ]);
});

test("CLI emits only the JSON contract and exposes its version", async () => {
  const analyzer = resolve(here, "..", "analyze.mjs");
  const [{ stdout: version }, { stdout }] = await Promise.all([
    execute(process.execPath, [analyzer, "--version"]),
    execute(process.execPath, [analyzer, fixture("static")]),
  ]);

  assert.equal(version.trim(), ANALYZER_VERSION);
  const result = JSON.parse(stdout);
  assert.deepEqual(Object.keys(result), [
    "analyzer_version",
    "analyzed_files",
    "unsupported_files",
    "edges",
    "gaps",
  ]);
  assert.ok(result.analyzed_files.every((path) => !path.startsWith("/")));
  assert.ok(result.edges.every(({ source_path, target_path }) => !source_path.startsWith("/") && !target_path.startsWith("/")));
});

test("inherited paths use a child's explicit baseUrl just as TypeScript does", async (context) => {
  const root = await mkdtemp(resolve(tmpdir(), "scope-dependency-path-origin-"));
  context.after(() => rm(root, { recursive: true }));
  for (const directory of ["config/src", "app/src"]) {
    await mkdir(resolve(root, directory), { recursive: true });
  }
  await writeFile(resolve(root, "config/base.json"), JSON.stringify({
    compilerOptions: { paths: { "@x/*": ["src/*"] } },
  }));
  const configPath = resolve(root, "app/tsconfig.json");
  await writeFile(configPath, JSON.stringify({
    extends: "../config/base.json", compilerOptions: { baseUrl: "." },
  }));
  for (const directory of ["config/src", "app/src"]) {
    await writeFile(resolve(root, directory, "value.ts"), "export const value = 1;\n");
  }
  const source = resolve(root, "app/main.ts");
  await writeFile(source, "import { value } from '@x/value'; export { value };\n");
  const config = ts.readConfigFile(configPath, ts.sys.readFile);
  const parsed = ts.parseJsonConfigFileContent(config.config, ts.sys, dirname(configPath), {}, configPath);
  const resolved = ts.resolveModuleName("@x/value", source, parsed.options, ts.sys).resolvedModule;
  assert.equal(resolved.resolvedFileName, resolve(root, "app/src/value.ts"));
  const result = await analyzeSnapshot(root);
  assert.deepEqual(result.gaps, []);
  assert.deepEqual(result.edges, [{ source_path: "app/main.ts", target_path: "app/src/value.ts", kind: "import" }]);
});
