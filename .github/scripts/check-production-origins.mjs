#!/usr/bin/env node

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const MANIFEST = ".github/deployment-services.json";

const ORIGINS = {
  api: {
    label: "production API origin",
    read: (manifest) => manifest?.releaseAvailability?.production?.apiOrigin,
    pointer: "releaseAvailability.production.apiOrigin",
  },
  cli: {
    label: "production CLI installer origin",
    read: (manifest) => manifest?.environments?.production?.cliPublicOrigin,
    pointer: "environments.production.cliPublicOrigin",
  },
};

const REPLICAS = [
  {
    path: "cli/src/api.rs",
    label: "DEFAULT_API_URL",
    origin: "api",
    pattern: /const DEFAULT_API_URL: &str = "([^"]+)";/g,
    count: 1,
  },
  {
    path: "crates/scope-api-contract/src/cli_compatibility.rs",
    label: "advertised installer URL",
    origin: "cli",
    pattern: /(https:\/\/[^\s"`]+)\/install\.sh/g,
    count: 2,
  },
];

const ORIGIN_FILES = [MANIFEST, ...new Set(REPLICAS.map(({ path }) => path))];

export function readOriginFiles(root = ".") {
  return Object.fromEntries(
    ORIGIN_FILES.map((path) => [path, readFileSync(resolve(root, path), "utf8")]),
  );
}

function originOf(value) {
  try {
    const url = new URL(value);
    return url.protocol === "https:" ? url.origin : null;
  } catch {
    return null;
  }
}

export function validateProductionOrigins(files) {
  let manifest;
  try {
    manifest = JSON.parse(files[MANIFEST]);
  } catch {
    return [`${MANIFEST}: is not valid JSON`];
  }

  const expected = {};
  const errors = [];
  for (const [key, { label, read, pointer }] of Object.entries(ORIGINS)) {
    const declared = read(manifest);
    const origin = typeof declared === "string" ? originOf(declared) : null;
    if (origin === null || origin !== declared) {
      errors.push(`${MANIFEST}: ${pointer} must declare the ${label} as a bare https origin`);
      continue;
    }
    expected[key] = declared;
  }
  if (errors.length > 0) return errors;

  for (const replica of REPLICAS) {
    const content = files[replica.path];
    if (content === undefined) {
      errors.push(`${replica.path}: file is missing`);
      continue;
    }

    const found = [...content.matchAll(replica.pattern)].map((match) => match[1]);
    if (found.length !== replica.count) {
      errors.push(
        `${replica.path}: expected ${replica.count} ${replica.label} occurrence(s), found ${found.length}`,
      );
      continue;
    }

    const want = expected[replica.origin];
    const mismatches = [...new Set(found.filter((value) => originOf(value) !== want))];
    if (mismatches.length > 0) {
      errors.push(
        `${replica.path}: ${replica.label} must match ${MANIFEST} ${ORIGINS[replica.origin].pointer} (${want}); found ${mismatches.join(", ")}`,
      );
    }
  }

  return errors;
}

function main() {
  const files = readOriginFiles();
  const errors = validateProductionOrigins(files);
  if (errors.length > 0) {
    process.stderr.write(`Production origins are out of sync:\n- ${errors.join("\n- ")}\n`);
    process.exitCode = 1;
    return;
  }

  const manifest = JSON.parse(files[MANIFEST]);
  const origins = Object.values(ORIGINS)
    .map(({ read }) => read(manifest))
    .join(", ");
  process.stdout.write(`Production origin replicas match ${origins}.\n`);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) main();
