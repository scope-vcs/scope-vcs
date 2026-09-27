import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import test from "node:test";

import { isScopeManagedPath, readScopeManagedFile } from "./scope-managed-files.mjs";

function checkout(t) {
  const root = mkdtempSync(resolve(tmpdir(), "scope-managed-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  return root;
}

test("every .scope path except the rules is Scope-managed", () => {
  assert.equal(isScopeManagedPath(".scope/runs/checks.yml"), true);
  assert.equal(isScopeManagedPath(".scope/images/checks/Dockerfile"), true);
  assert.equal(isScopeManagedPath(".scope/RULES.md"), false);
  assert.equal(isScopeManagedPath("deploy/railway/web.Dockerfile"), false);
  assert.throws(() => readScopeManagedFile("Cargo.toml"), /not Scope-managed/);
});

test("a present Scope-managed file is read", (t) => {
  const root = checkout(t);
  mkdirSync(resolve(root, ".scope/runs"), { recursive: true });
  writeFileSync(resolve(root, ".scope/runs/checks.yml"), "name: checks\n");
  assert.equal(readScopeManagedFile(".scope/runs/checks.yml", { root, env: {} }), "name: checks\n");
});

test("a public projection skips a missing Scope-managed file and says so", (t) => {
  const root = checkout(t);
  const write = t.mock.method(process.stderr, "write", () => true);
  assert.equal(readScopeManagedFile(".scope/runs/checks.yml", { root, env: {} }), undefined);
  assert.deepEqual(write.mock.calls.map((call) => call.arguments[0]), [
    "Skipped .scope/runs/checks.yml: Scope keeps it private, so this checkout does not include it.\n",
  ]);
});

test("GitHub Actions fails on a missing Scope-managed file", (t) => {
  const root = checkout(t);
  assert.throws(
    () => readScopeManagedFile(".scope/runs/checks.yml", { root, env: { GITHUB_ACTIONS: "true" } }),
    { code: "ENOENT" },
  );
});
