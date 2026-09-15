#!/usr/bin/env node

import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

// Pull requests verify the native cargo targets on Blacksmith runners. The full
// matrix, including GitHub-hosted macOS Intel and Windows ARM64, runs on release.
const PULL_REQUEST_TARGETS = new Set([
  "x86_64-unknown-linux-gnu",
  "aarch64-apple-darwin",
  "x86_64-pc-windows-msvc",
]);

export function selectCliDistributionTargets(configuration, mode) {
  if (!Array.isArray(configuration?.targets) || configuration.targets.length === 0) {
    throw new Error("CLI distribution configuration must contain a non-empty targets array");
  }
  if (typeof configuration.node_version !== "string") {
    throw new Error("CLI distribution configuration must pin node_version");
  }
  if (mode !== "pull-request" && mode !== "release") {
    throw new Error(`Unknown CLI distribution mode: ${mode}`);
  }

  const targets = mode === "release"
    ? configuration.targets
    : configuration.targets.filter(({ triple }) => PULL_REQUEST_TARGETS.has(triple));
  if (targets.length === 0) {
    throw new Error(`CLI distribution mode ${mode} selected no targets`);
  }

  return {
    include: targets.map(({ triple, ...target }) => ({
      ...target,
      target: triple,
      node_version: configuration.node_version,
    })),
  };
}

function argument(name, fallback = "") {
  const index = process.argv.indexOf(name);
  return index === -1 ? fallback : process.argv[index + 1] ?? fallback;
}

function main() {
  const targetsPath = argument("--targets", "cli/distribution/targets.json");
  const mode = argument("--mode");
  const configuration = JSON.parse(readFileSync(targetsPath, "utf8"));
  process.stdout.write(`${JSON.stringify(selectCliDistributionTargets(configuration, mode))}\n`);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) main();
