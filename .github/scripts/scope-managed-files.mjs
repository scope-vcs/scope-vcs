import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

// Scope keeps every .scope path except RULES.md private, so public projections
// omit these files. Checks that read them skip them there. GitHub Actions always
// checks out the complete tree, so a missing file fails instead of skipping.
export function isScopeManagedPath(path) {
  return path.startsWith(".scope/") && path !== ".scope/RULES.md";
}

export function readScopeManagedFile(path, { root = ".", env = process.env } = {}) {
  if (!isScopeManagedPath(path)) throw new Error(`${path} is not Scope-managed`);
  const absolute = resolve(root, path);
  if (existsSync(absolute) || env.GITHUB_ACTIONS === "true") return readFileSync(absolute, "utf8");
  process.stderr.write(`Skipped ${path}: Scope keeps it private, so this checkout does not include it.\n`);
  return undefined;
}
