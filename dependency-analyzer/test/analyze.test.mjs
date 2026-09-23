import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdtemp, mkdir, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import test from "node:test";

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
  const result = await analyzeSnapshot(root);
  assert.deepEqual(result.gaps, []);
  assert.deepEqual(result.edges, [{ source_path: "app/main.ts", target_path: "app/src/value.ts", kind: "import" }]);
});

test("resolves package self exports and import maps without installed packages", async (context) => {
  const root = await mkdtemp(resolve(tmpdir(), "scope-dependency-package-map-"));
  context.after(() => rm(root, { recursive: true }));
  await mkdir(resolve(root, "src/private"), { recursive: true });
  await writeFile(resolve(root, "package.json"), JSON.stringify({
    name: "@acme/app",
    exports: { "./public": "./src/private/value.ts" },
    imports: { "#internal": "./src/private/value.ts" },
  }));
  await writeFile(resolve(root, "src/private/value.ts"), "export const value = 1;\n");
  await writeFile(resolve(root, "src/main.ts"), [
    "import { value } from '@acme/app/public';",
    "export { value as internal } from '#internal';",
    "",
  ].join("\n"));
  const result = await analyzeSnapshot(root);
  assert.deepEqual(result.gaps, []);
  assert.deepEqual(result.edges, [
    { source_path: "src/main.ts", target_path: "src/private/value.ts", kind: "import" },
    { source_path: "src/main.ts", target_path: "src/private/value.ts", kind: "re-export" },
  ]);
});

test("rejects extended configs outside the snapshot and keeps relative edges", async (context) => {
  const parent = await mkdtemp(resolve(tmpdir(), "scope-dependency-config-escape-"));
  context.after(() => rm(parent, { recursive: true }));
  const root = resolve(parent, "repo");
  await mkdir(resolve(root, "src"), { recursive: true });
  await writeFile(resolve(parent, "outside.json"), JSON.stringify({ compilerOptions: { paths: { "@secret": ["../outside.ts"] } } }));
  await writeFile(resolve(root, "tsconfig.json"), '{ // JSONC is accepted\n "extends": "../outside.json",\n}\n');
  await writeFile(resolve(root, "src/main.ts"), "import './value';\n");
  await writeFile(resolve(root, "src/value.ts"), "export const value = 1;\n");
  const result = await analyzeSnapshot(root);
  assert.deepEqual(result.edges, [
    { source_path: "src/main.ts", target_path: "src/value.ts", kind: "side-effect-import" },
  ]);
  assert.ok(result.gaps.some(({ path, reason }) => path === "tsconfig.json" && reason.includes("escapes snapshot")));
});

test("uses import, require, and types package export conditions", async (context) => {
  const root = await mkdtemp(resolve(tmpdir(), "scope-dependency-conditions-"));
  context.after(() => rm(root, { recursive: true }));
  await mkdir(resolve(root, "src/private"), { recursive: true });
  await writeFile(resolve(root, "package.json"), JSON.stringify({
    name: "@acme/app",
    exports: { "./conditions": {
      types: "./src/private/types.d.ts",
      import: "./src/private/esm.ts",
      require: "./src/private/cjs.cts",
    } },
  }));
  for (const file of ["types.d.ts", "esm.ts", "cjs.cts"]) {
    await writeFile(resolve(root, "src/private", file), "export const value = 1;\n");
  }
  await writeFile(resolve(root, "src/import.ts"), "import { value } from '@acme/app/conditions';\n");
  await writeFile(resolve(root, "src/esm.mts"), "import { value } from '@acme/app/conditions';\n");
  await writeFile(resolve(root, "src/type.ts"), "import type { value } from '@acme/app/conditions';\n");
  await writeFile(resolve(root, "src/require.cts"), "import value = require('@acme/app/conditions');\n");
  await writeFile(resolve(root, "src/regular.cts"), "import { value } from '@acme/app/conditions';\n");
  const result = await analyzeSnapshot(root);
  assert.deepEqual(result.gaps, []);
  assert.deepEqual(result.edges, [
    { source_path: "src/esm.mts", target_path: "src/private/esm.ts", kind: "import" },
    { source_path: "src/import.ts", target_path: "src/private/esm.ts", kind: "import" },
    { source_path: "src/regular.cts", target_path: "src/private/cjs.cts", kind: "import" },
    { source_path: "src/require.cts", target_path: "src/private/cjs.cts", kind: "require" },
    { source_path: "src/type.ts", target_path: "src/private/types.d.ts", kind: "type-import" },
  ]);
});

test("rejects path aliases that point outside the snapshot", async (context) => {
  const root = await mkdtemp(resolve(tmpdir(), "scope-dependency-alias-escape-"));
  context.after(() => rm(root, { recursive: true }));
  await mkdir(resolve(root, "src"));
  await writeFile(resolve(root, "tsconfig.json"), JSON.stringify({
    compilerOptions: { baseUrl: ".", paths: { "@outside/*": ["../outside/*"] } },
  }));
  await writeFile(resolve(root, "src/main.ts"), "export const ok = true;\n");
  const result = await analyzeSnapshot(root);
  assert.ok(result.gaps.some(({ path, reason }) => path === "tsconfig.json" && reason.includes("escapes its root")));
});

test("ignores installed external packages in a snapshot's parent directory", async (context) => {
  const parent = await mkdtemp(resolve(tmpdir(), "scope-dependency-parent-packages-"));
  context.after(() => rm(parent, { recursive: true }));
  const root = resolve(parent, "repo");
  await mkdir(resolve(parent, "node_modules/external"), { recursive: true });
  await mkdir(resolve(root, "src"), { recursive: true });
  await writeFile(resolve(parent, "node_modules/external/package.json"), JSON.stringify({ main: "index.js" }));
  await writeFile(resolve(parent, "node_modules/external/index.js"), "module.exports = 1;\n");
  await writeFile(resolve(root, "src/main.ts"), "import value from 'external';\n");
  const result = await analyzeSnapshot(root);
  assert.deepEqual(result.edges, []);
  assert.deepEqual(result.gaps, []);
});

test("bundler query imports retain the underlying source edge", async (context) => {
  const root = await mkdtemp(resolve(tmpdir(), "scope-dependency-worker-query-"));
  context.after(() => rm(root, { recursive: true }));
  await mkdir(resolve(root, "src"));
  await writeFile(resolve(root, "src/main.ts"), "export const worker = () => import('./worker.ts?worker');\n");
  await writeFile(resolve(root, "src/worker.ts"), "export const value = 1;\n");
  const result = await analyzeSnapshot(root);
  assert.deepEqual(result.gaps, []);
  assert.deepEqual(result.edges, [
    { source_path: "src/main.ts", target_path: "src/worker.ts", kind: "dynamic-import" },
  ]);
});

test("rootDirs overlays relative imports without discarding inherited aliases", async (context) => {
  const root = await mkdtemp(resolve(tmpdir(), "scope-root-dirs-"));
  context.after(() => rm(root, { recursive: true }));
  const files = {
    "config/base.json": JSON.stringify({ compilerOptions: {
      baseUrl: "..", paths: { "@private/*": ["private/*"] }, rootDirs: ["../src", "../src/public", "../generated"],
    } }),
    "config/strict.json": JSON.stringify({ compilerOptions: { strict: true } }),
    "tsconfig.json": JSON.stringify({ extends: ["./config/base.json", "./config/strict.json"] }),
    "src/public/entry.ts": "import '../shared'; import '@private/value'; import './nested'; import './own'; import './style.css';\n",
    "generated/shared.ts": "export const generated = true;\n",
    "generated/nested.ts": "export {};\n",
    "generated/style.css.d.ts": "export {};\n",
    "generated/own.ts": "export {};\n",
    "src/public/own.ts": "export {};\n",
    "private/value.ts": "export const secret = true;\n",
  };
  for (const [path, content] of Object.entries(files)) {
    await mkdir(dirname(resolve(root, path)), { recursive: true });
    await writeFile(resolve(root, path), content);
  }
  const result = await analyzeSnapshot(root);
  assert.deepEqual(result.gaps, []);
  assert.deepEqual(edgesFrom(result, "src/public/entry.ts").map(({ target_path }) => target_path), [
    "generated/nested.ts", "generated/shared.ts", "generated/style.css.d.ts", "private/value.ts", "src/public/own.ts",
  ]);
});

test("moduleSuffixes preserves aliases, extension priority, explicit imports and directory indexes", async (context) => {
  const root = await mkdtemp(resolve(tmpdir(), "scope-module-suffixes-"));
  context.after(() => rm(root, { recursive: true }));
  const files = {
    "config/base.json": JSON.stringify({ compilerOptions: { moduleSuffixes: [".native", ""] } }),
    "tsconfig.json": JSON.stringify({ extends: "./config/base.json", compilerOptions: {
      baseUrl: ".", paths: { "@private/*": ["private/*"] },
    } }),
    "src/entry.ts": "import '@private/value'; import '@private/explicit.js'; import '@private/folder'; import '@private/priority';\n",
    "private/value.ts": "export {};\n",
    "private/value.native.ts": "export {};\n",
    "private/explicit.native.ts": "export {};\n",
    "private/folder/index.native.ts": "export {};\n",
    "private/priority.ts": "export {};\n",
    "private/priority.native.tsx": "export {};\n",
  };
  for (const [path, content] of Object.entries(files)) {
    await mkdir(dirname(resolve(root, path)), { recursive: true });
    await writeFile(resolve(root, path), content);
  }
  const result = await analyzeSnapshot(root);
  assert.deepEqual(result.gaps, []);
  assert.deepEqual(edgesFrom(result, "src/entry.ts").map(({ target_path }) => target_path), [
    "private/explicit.native.ts", "private/folder/index.native.ts", "private/priority.ts", "private/value.native.ts",
  ]);
});

test("moduleSuffixes without an empty suffix does not silently use an unsuffixed file", async (context) => {
  const root = await mkdtemp(resolve(tmpdir(), "scope-required-suffix-"));
  context.after(() => rm(root, { recursive: true }));
  await writeFile(resolve(root, "tsconfig.json"), JSON.stringify({ compilerOptions: { moduleSuffixes: [".native"] } }));
  await writeFile(resolve(root, "entry.ts"), "import './value';\n");
  await writeFile(resolve(root, "value.ts"), "export {};\n");
  const result = await analyzeSnapshot(root);
  assert.deepEqual(result.edges, []);
  assert.deepEqual(result.gaps, [{ path: "entry.ts", reason: "unresolved side-effect-import: ./value" }]);
  await writeFile(resolve(root, "tsconfig.json"), JSON.stringify({ compilerOptions: { moduleSuffixes: [] } }));
  const defaultResult = await analyzeSnapshot(root);
  assert.deepEqual(defaultResult.gaps, []);
  assert.equal(defaultResult.edges[0].target_path, "value.ts");
});

test("rootDirs cannot resolve outside the snapshot", async (context) => {
  const root = await mkdtemp(resolve(tmpdir(), "scope-root-dirs-escape-"));
  context.after(() => rm(root, { recursive: true }));
  await writeFile(resolve(root, "tsconfig.json"), JSON.stringify({ compilerOptions: { rootDirs: [".", "../outside"] } }));
  await writeFile(resolve(root, "entry.ts"), "export {};\n");
  const result = await analyzeSnapshot(root);
  assert.ok(result.gaps.some(({ path, reason }) => path === "tsconfig.json" && reason.startsWith("invalid TypeScript config:")));
});
