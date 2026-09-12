import { opendir, readFile, realpath, stat } from "node:fs/promises";
import { extname, join, relative, resolve, sep } from "node:path";

import {
  IGNORED_DIRECTORY_NAMES,
  LIMITS,
  SUPPORTED_SOURCE_EXTENSIONS,
  UNSUPPORTED_SOURCE_EXTENSIONS,
} from "./constants.mjs";

export class SnapshotError extends Error {}

function repoPath(parent, name) {
  return parent ? `${parent}/${name}` : name;
}

function assertWithinRoot(root, candidate) {
  const difference = relative(root, candidate);
  if (difference === ".." || difference.startsWith(`..${sep}`)) {
    throw new SnapshotError("snapshot path escapes its root");
  }
}

function assertPathBound(path) {
  if (Buffer.byteLength(path) > LIMITS.maxPathBytes) {
    throw new SnapshotError(`path length limit exceeded (${LIMITS.maxPathBytes} bytes)`);
  }
}

function isResolutionConfig(path) {
  const name = path.slice(path.lastIndexOf("/") + 1);
  return /^(?:ts|js)config(?:\.[^.\/]+)*\.json$/u.test(name);
}

export async function inventorySnapshot(inputRoot) {
  const requestedRoot = resolve(inputRoot);
  let root;
  try {
    root = await realpath(requestedRoot);
  } catch (error) {
    throw new SnapshotError(`cannot read snapshot directory: ${error.message}`);
  }

  const rootStat = await stat(root);
  if (!rootStat.isDirectory()) {
    throw new SnapshotError("snapshot path is not a directory");
  }

  const allFiles = new Set();
  const configs = [];
  const sources = [];
  const unsupportedFiles = [];
  const directories = [{ absolute: root, path: "" }];
  let totalInputBytes = 0;

  while (directories.length > 0) {
    const directory = directories.pop();
    assertWithinRoot(root, directory.absolute);

    let handle;
    try {
      handle = await opendir(directory.absolute);
      for await (const entry of handle) {
        const path = repoPath(directory.path, entry.name);
        assertPathBound(path);

        if (entry.isSymbolicLink()) {
          throw new SnapshotError(`symbolic links are not analyzed: ${path}`);
        }
        if (entry.isDirectory()) {
          if (!IGNORED_DIRECTORY_NAMES.has(entry.name)) {
            directories.push({ absolute: join(directory.absolute, entry.name), path });
          }
          continue;
        }
        if (!entry.isFile()) continue;

        allFiles.add(path);
        if (allFiles.size > LIMITS.maxFiles) {
          throw new SnapshotError(`file limit exceeded (${LIMITS.maxFiles})`);
        }

        const extension = extname(entry.name).toLowerCase();
        const resolutionConfig = isResolutionConfig(path);
        if (resolutionConfig) configs.push(path);

        if (SUPPORTED_SOURCE_EXTENSIONS.has(extension) || extension === ".json" || extension === ".jsonc") {
          const metadata = await stat(join(directory.absolute, entry.name));
          if (metadata.size > LIMITS.maxSourceBytes) {
            throw new SnapshotError(
              `analyzer input file size limit exceeded (${LIMITS.maxSourceBytes} bytes): ${path}`,
            );
          }
          totalInputBytes += metadata.size;
          if (totalInputBytes > LIMITS.maxTotalSourceBytes) {
            throw new SnapshotError(
              `total analyzer input size limit exceeded (${LIMITS.maxTotalSourceBytes} bytes)`,
            );
          }
        }

        if (SUPPORTED_SOURCE_EXTENSIONS.has(extension)) {
          sources.push(path);
        } else if (UNSUPPORTED_SOURCE_EXTENSIONS.has(extension)) {
          unsupportedFiles.push(path);
          if (unsupportedFiles.length > LIMITS.maxUnsupportedSources) {
            throw new SnapshotError(
              `unsupported source file limit exceeded (${LIMITS.maxUnsupportedSources})`,
            );
          }
        }
      }
    } catch (error) {
      if (handle) await handle.close().catch(() => {});
      if (error instanceof SnapshotError) throw error;
      throw new SnapshotError(`cannot inventory snapshot: ${error.message}`);
    }
  }

  return {
    allFiles,
    configs: configs.sort(),
    root,
    sources: sources.sort(),
    unsupportedFiles: unsupportedFiles.sort(),
  };
}

// Package names declared inside the snapshot identify workspace packages whose
// imports must resolve within the repository rather than from a registry.
export async function internalPackageNames(root, allFiles) {
  const names = new Set();
  for (const path of allFiles) {
    if (path !== "package.json" && !path.endsWith("/package.json")) continue;
    let manifest;
    try {
      manifest = JSON.parse(await readFile(absoluteSnapshotPath(root, path), "utf8"));
    } catch {
      continue;
    }
    if (typeof manifest?.name === "string" && manifest.name.length > 0) names.add(manifest.name);
  }
  return names;
}

export function absoluteSnapshotPath(root, path) {
  const absolute = resolve(root, path);
  assertWithinRoot(root, absolute);
  return absolute;
}
