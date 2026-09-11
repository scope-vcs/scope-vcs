import { lstat, realpath } from "node:fs/promises";
import { posix, relative, resolve, sep } from "node:path";

import { cruise } from "dependency-cruiser";

import { absoluteSnapshotPath } from "./snapshot.mjs";

function normalizedSourcePath(source) {
  return posix.normalize(source.replaceAll("\\", "/")).replace(/^\.\//u, "");
}

function isOutside(root, candidate) {
  const difference = relative(root, candidate);
  return difference === ".." || difference.startsWith(`..${sep}`);
}

function fallbackKind(dependency) {
  if (dependency.dynamic || dependency.dependencyTypes.includes("dynamic-import")) {
    return "dynamic-import";
  }
  if (dependency.typeOnly || dependency.dependencyTypes.includes("type-only")) {
    return dependency.dependencyTypes.includes("export") ? "type-re-export" : "type-import";
  }
  if (dependency.dependencyTypes.includes("export")) return "re-export";
  if (dependency.moduleSystem === "cjs") return "require";
  return "import";
}

function sourceSpecifierFor(dependency) {
  if (!dependency.protocol || dependency.module.startsWith(dependency.protocol)) {
    return dependency.module;
  }
  return `${dependency.protocol}${dependency.module}`;
}

function kindsForDependency(references, dependency) {
  const sourceSpecifier = sourceSpecifierFor(dependency);
  const kinds = new Set(
    references
      .filter(({ specifier }) => specifier === sourceSpecifier)
      .map(({ kind }) => kind),
  );
  if (kinds.size === 0) kinds.add(fallbackKind(dependency));
  return [...kinds];
}

function isExternal(dependency) {
  return dependency.coreModule || dependency.dependencyTypes.some((type) =>
    type === "core" || type.startsWith("npm"),
  );
}

function isBareSpecifier(specifier) {
  return !specifier.startsWith(".") && !specifier.startsWith("/") && !specifier.startsWith("#");
}

function packageNameOf(specifier) {
  const segments = specifier.split("/");
  return specifier.startsWith("@") ? segments.slice(0, 2).join("/") : segments[0];
}

function aliasMatcher(paths) {
  const patterns = Object.keys(paths ?? {}).map((pattern) => new RegExp(
    `^${pattern.split("*").map((part) => part.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&")).join(".*")}$`,
    "u",
  ));
  return (specifier) => patterns.some((pattern) => pattern.test(specifier));
}

// Snapshots never contain installed packages, so a bare specifier that does not
// resolve is an external dependency unless the repository itself claims the
// name through a path alias, a workspace package, or a package import map.
function isUnvendoredExternal(dependency, matchesAlias, internalPackageNames) {
  const specifier = dependency.module;
  return isBareSpecifier(specifier)
    && !matchesAlias(specifier)
    && !internalPackageNames.has(packageNameOf(specifier));
}

async function validateInternalTarget(root, targetPath) {
  const absolute = absoluteSnapshotPath(root, targetPath);
  const metadata = await lstat(absolute);
  if (!metadata.isFile()) return false;
  const canonical = await realpath(absolute);
  return !isOutside(root, canonical);
}

export async function cruiseGroup({
  allFiles,
  configPath,
  internalPackageNames,
  referencesBySource,
  root,
  sources,
  transpileOptions,
}) {
  const sourceSet = new Set(sources);
  const matchesAlias = aliasMatcher(transpileOptions?.tsConfig?.options?.paths);
  const options = {
    baseDir: root,
    combinedDependencies: true,
    doNotFollow: "^(?:\\.\\./|/)|(^|/)node_modules/",
    parser: "tsc",
    preserveSymlinks: true,
    tsPreCompilationDeps: true,
    ...(configPath ? { tsConfig: { fileName: absoluteSnapshotPath(root, configPath) } } : {}),
  };

  const cruiseResult = await cruise(
    sources,
    options,
    { bustTheCache: true },
    transpileOptions,
  );

  if (typeof cruiseResult.output !== "object" || !Array.isArray(cruiseResult.output.modules)) {
    throw new Error("dependency-cruiser returned an invalid result");
  }

  const edges = [];
  const gaps = [];
  const seenSpecifiers = new Map(sources.map((source) => [source, new Set()]));

  for (const module of cruiseResult.output.modules) {
    const sourcePath = normalizedSourcePath(module.source);
    if (!sourceSet.has(sourcePath)) continue;
    const references = referencesBySource.get(sourcePath) ?? [];

    for (const dependency of module.dependencies) {
      seenSpecifiers.get(sourcePath).add(sourceSpecifierFor(dependency));
      const kinds = kindsForDependency(references, dependency);

      if (dependency.couldNotResolve) {
        if (isUnvendoredExternal(dependency, matchesAlias, internalPackageNames)) continue;
        for (const kind of kinds) {
          gaps.push({
            path: sourcePath,
            reason: `unresolved ${kind}: ${dependency.module}`,
          });
        }
        continue;
      }
      if (isExternal(dependency)) continue;

      const resolvedTarget = resolve(root, dependency.resolved);
      if (isOutside(root, resolvedTarget)) {
        gaps.push({
          path: sourcePath,
          reason: `resolved target escapes snapshot: ${dependency.module}`,
        });
        continue;
      }

      const targetPath = normalizedSourcePath(relative(root, resolvedTarget));
      if (targetPath.split("/").includes("node_modules")) continue;
      if (!allFiles.has(targetPath)) {
        gaps.push({
          path: sourcePath,
          reason: `resolved target is absent from snapshot: ${dependency.module}`,
        });
        continue;
      }

      try {
        if (!(await validateInternalTarget(root, targetPath))) {
          gaps.push({ path: sourcePath, reason: `resolved target is not a regular file: ${dependency.module}` });
          continue;
        }
      } catch {
        gaps.push({ path: sourcePath, reason: `resolved target is unreadable: ${dependency.module}` });
        continue;
      }

      for (const kind of kinds) edges.push({ source_path: sourcePath, target_path: targetPath, kind });
    }
  }

  for (const source of sources) {
    const seen = seenSpecifiers.get(source);
    for (const reference of referencesBySource.get(source) ?? []) {
      if (!seen.has(reference.specifier)) {
        gaps.push({
          path: source,
          reason: `dependency analyzer omitted ${reference.kind}: ${reference.specifier}`,
        });
      }
    }
  }

  return { edges, gaps };
}
