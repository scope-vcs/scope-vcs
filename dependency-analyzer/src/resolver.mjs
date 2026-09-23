import { lstat, realpath } from "node:fs/promises";
import { dirname, posix, relative, resolve, sep } from "node:path";

import enhancedResolve from "enhanced-resolve";

import { absoluteSnapshotPath } from "./snapshot.mjs";

function isOutside(root, candidate) {
  const difference = relative(root, candidate);
  return difference === ".." || difference.startsWith(`..${sep}`);
}

function packageNameOf(specifier) {
  const segments = specifier.split("/");
  return specifier.startsWith("@") ? segments.slice(0, 2).join("/") : segments[0];
}

function aliasMatcher(paths) {
  const patterns = Object.keys(paths).map((pattern) => new RegExp(
    `^${pattern.split("*").map((part) => part.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&")).join(".*")}$`,
    "u",
  ));
  return (specifier) => patterns.some((pattern) => pattern.test(specifier));
}

function isBareSpecifier(specifier) {
  return !specifier.startsWith(".") && !specifier.startsWith("/") && !specifier.startsWith("#");
}

function createResolver(configPath, kind, source) {
  const typeOnly = kind === "type-import" || kind === "type-re-export";
  const commonJsSource = source.endsWith(".cts") || source.endsWith(".cjs");
  const requireMode = kind === "require" || commonJsSource && kind !== "dynamic-import";
  const conditions = typeOnly ? ["types", requireMode ? "require" : "import", "node", "default"]
    : requireMode ? ["require", "node", "default"] : ["import", "node", "default"];
  return enhancedResolve.create.promise({
    conditionNames: conditions,
    exportsFields: ["exports"],
    importsFields: ["imports"],
    extensions: [".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs", ".json"],
    extensionAlias: { ".js": [".ts", ".tsx", ".js"], ".mjs": [".mts", ".mjs"], ".cjs": [".cts", ".cjs"] },
    mainFields: typeOnly ? ["types", "module", "main"] : ["module", "main"],
    mainFiles: ["index"],
    symlinks: false,
    ...(configPath ? { tsconfig: { configFile: configPath } } : {}),
  });
}

export async function resolveGroup({ allFiles, configPath, internalPackageNames, paths, referencesBySource, root, sources }) {
  const matchesAlias = aliasMatcher(paths);
  const resolvers = new Map();
  const edges = [];
  const gaps = [];
  for (const source of sources) {
    for (const { kind, specifier } of referencesBySource.get(source) ?? []) {
      if (specifier.startsWith("node:")) continue;
      const unclaimedBare = isBareSpecifier(specifier)
        && !matchesAlias(specifier)
        && !internalPackageNames.has(packageNameOf(specifier));
      const resolverKey = `${kind}:${source.endsWith(".cts") || source.endsWith(".cjs")}`;
      if (!resolvers.has(resolverKey)) resolvers.set(resolverKey, createResolver(configPath, kind, source));
      let target;
      try {
        target = await resolvers.get(resolverKey)(dirname(absoluteSnapshotPath(root, source)), specifier);
      } catch {
        target = null;
      }
      if (!target) {
        if (unclaimedBare) continue;
        const candidate = specifier.startsWith(".") ? resolve(root, dirname(source), specifier) : null;
        if (candidate && isOutside(root, candidate)) {
          gaps.push({ path: source, reason: `resolved target escapes snapshot: ${specifier}` });
        } else {
          gaps.push({ path: source, reason: `unresolved ${kind}: ${specifier}` });
        }
        continue;
      }
      // Resolver preserves bundler resource queries in its result. The edge
      // still points at the source file that the query transforms.
      const resolvedFile = target.split("?")[0];
      if (isOutside(root, resolvedFile)) {
        if (unclaimedBare) continue;
        gaps.push({ path: source, reason: `resolved target escapes snapshot: ${specifier}` });
        continue;
      }
      const targetPath = posix.normalize(relative(root, resolvedFile).replaceAll("\\", "/"));
      if (targetPath.split("/").includes("node_modules")) continue;
      if (!allFiles.has(targetPath)) {
        gaps.push({ path: source, reason: `resolved target is absent from snapshot: ${specifier}` });
        continue;
      }
      try {
        const metadata = await lstat(resolvedFile);
        const canonical = await realpath(resolvedFile);
        if (!metadata.isFile() || isOutside(root, canonical)) {
          gaps.push({ path: source, reason: `resolved target is not a regular file: ${specifier}` });
          continue;
        }
      } catch {
        gaps.push({ path: source, reason: `resolved target is unreadable: ${specifier}` });
        continue;
      }
      edges.push({ source_path: source, target_path: targetPath, kind });
    }
  }
  return { edges, gaps };
}
