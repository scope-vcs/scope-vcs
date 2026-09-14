#!/usr/bin/env node

import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

export function selectCliDistributionTargets(configuration) {
  if (!Array.isArray(configuration?.targets) || configuration.targets.length === 0) {
    throw new Error("CLI distribution configuration must contain a non-empty targets array");
  }
  if (typeof configuration.node_version !== "string") {
    throw new Error("CLI distribution configuration must pin node_version");
  }

  return {
    include: configuration.targets.map(({ triple, ...target }) => ({
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
  const configuration = JSON.parse(readFileSync(targetsPath, "utf8"));
  process.stdout.write(`${JSON.stringify(selectCliDistributionTargets(configuration))}\n`);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) main();
