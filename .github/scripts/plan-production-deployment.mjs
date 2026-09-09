#!/usr/bin/env node

import { execFileSync } from "node:child_process";
import { appendFileSync, readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";

export const COMPONENTS = [
  "checks-image",
  "cache",
  "run-worker",
  "media-worker",
  "git-router",
  "media-api",
  "api",
  "web",
  "cli-downloads",
];
const SELECTIONS = [...COMPONENTS, "cli-distribution"];

function matchesScope(path, scope) {
  return scope.files.includes(path) || scope.prefixes.some((prefix) => path.startsWith(prefix));
}

export function classifyChanges(manifest, paths, requestedScope = "changed") {
  const selection = Object.fromEntries(SELECTIONS.map((component) => [component, false]));

  if (requestedScope !== "changed") {
    if (requestedScope === "all") {
      return Object.fromEntries(SELECTIONS.map((component) => [component, true]));
    }
    if (!COMPONENTS.includes(requestedScope)) {
      throw new Error(`Unknown deployment scope: ${requestedScope}`);
    }
    selection[requestedScope] = true;
    if (requestedScope === "cli-downloads") selection["cli-distribution"] = true;
    return selection;
  }

  for (const path of paths) {
    if (matchesScope(path, manifest.changeScopes.all)) {
      return Object.fromEntries(SELECTIONS.map((component) => [component, true]));
    }
    for (const component of SELECTIONS) {
      if (matchesScope(path, manifest.changeScopes[component])) selection[component] = true;
    }
  }

  return selection;
}

// Every database writer and coupled application artifact must share a revision
// when the deployed API's migration inventory changes. Distribution-only releases
// remain independent, and an unchanged migration baseline preserves narrow scopes.
export function includeMigrationParticipants(selection, apiChanges) {
  const backend = ["cache", "run-worker", "media-worker", "git-router", "media-api", "api"];
  if (!backend.some((component) => selection[component])) return selection;
  if (Array.isArray(apiChanges) && !apiChanges.some((path) => path.startsWith("crates/scope-postgres/src/migrations/"))) return selection;
  return { ...selection, ...Object.fromEntries(SELECTIONS.filter((component) => component !== "checks-image").map((component) => [component, true])) };
}

export function planFromDeploymentProgress(manifest, pathsByComponent, requestedScope = "changed") {
  if (requestedScope !== "changed") return includeMigrationParticipants(classifyChanges(manifest, [], requestedScope), pathsByComponent.api);

  const selection = Object.fromEntries(COMPONENTS.map((component) => {
    const paths = pathsByComponent[component];
    if (!Array.isArray(paths)) return [component, true];
    return [component, classifyChanges(manifest, paths)[component]];
  }));
  const cliPaths = pathsByComponent["cli-downloads"];
  selection["cli-distribution"] = !Array.isArray(cliPaths)
    || classifyChanges(manifest, cliPaths)["cli-distribution"];
  return includeMigrationParticipants(selection, pathsByComponent.api);
}

function changedPaths(base, head, useMergeBase = true) {
  if (!base || /^0+$/.test(base)) {
    return execFileSync("git", ["ls-tree", "-r", "--name-only", head], { encoding: "utf8" })
      .split("\n")
      .filter(Boolean);
  }
  const range = useMergeBase ? `${base}...${head}` : `${base}..${head}`;
  return execFileSync("git", ["diff", "--name-only", range], { encoding: "utf8" })
    .split("\n")
    .filter(Boolean);
}

function pathsSinceSuccessfulDeployments(revisions, head) {
  return Object.fromEntries(COMPONENTS.map((component) => {
    const revision = revisions[component];
    if (typeof revision !== "string" || revision.length === 0) return [component, null];

    try {
      return [component, changedPaths(revision, head, false)];
    } catch (error) {
      process.stderr.write(
        `Could not compare ${component} deployment ${revision} with ${head}; selecting it conservatively.\n`,
      );
      return [component, null];
    }
  }));
}

function argument(name, fallback = "") {
  const index = process.argv.indexOf(name);
  return index === -1 ? fallback : process.argv[index + 1] ?? fallback;
}

function main() {
  const manifestPath = argument("--manifest", ".github/deployment-services.json");
  const requestedScope = argument("--scope", "changed");
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  const head = argument("--head", "HEAD");
  const deployedRevisionsJson = argument("--deployed-revisions");
  const usesDeploymentProgress = deployedRevisionsJson.length > 0;
  const paths = requestedScope === "changed" && !usesDeploymentProgress
    ? changedPaths(argument("--base"), head)
    : [];
  const pathsByComponent = usesDeploymentProgress
    ? pathsSinceSuccessfulDeployments(JSON.parse(deployedRevisionsJson), head)
    : null;
  const selection = pathsByComponent
    ? planFromDeploymentProgress(manifest, pathsByComponent, requestedScope)
    : classifyChanges(manifest, paths, requestedScope);
  const outputPath = process.env.GITHUB_OUTPUT;
  const summaryPath = process.env.GITHUB_STEP_SUMMARY;

  for (const [component, selected] of Object.entries(selection)) {
    const outputName = {
      "checks-image": "checks_image",
      "run-worker": "worker",
      "git-router": "router",
      "media-api": "media",
      "cli-downloads": "cli",
      "cli-distribution": "cli_distribution",
      "media-worker": "media_worker",
    }[component] ?? component;
    const line = `${outputName}=${selected}\n`;
    if (outputPath) appendFileSync(outputPath, line);
    else process.stdout.write(line);
  }

  if (summaryPath) {
    const selected = Object.entries(selection)
      .filter(([, value]) => value)
      .map(([component]) => component);
    appendFileSync(summaryPath, [
      "## Deployment plan",
      "",
      `Selected: ${selected.length > 0 ? selected.join(", ") : "none"}`,
      "",
      usesDeploymentProgress
        ? "Compared each component with its last successful production deployment"
        : paths.length > 0
          ? `Changed files considered: ${paths.length}`
          : `Requested scope: ${requestedScope}`,
      "",
    ].join("\n"));
  }
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) main();
