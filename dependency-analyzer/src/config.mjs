import { readFileSync } from "node:fs";
import { dirname, posix, resolve } from "node:path";

import { parse } from "jsonc-parser";

import { LIMITS } from "./constants.mjs";
import { absoluteSnapshotPath } from "./snapshot.mjs";

function configRank(path) {
  const name = posix.basename(path);
  if (name === "tsconfig.json") return 0;
  if (name === "jsconfig.json") return 1;
  if (name.startsWith("tsconfig.")) return 2;
  return 3;
}

export function groupSourcesByConfig(sources, configs) {
  const byDirectory = new Map();
  for (const config of configs) {
    const directory = posix.dirname(config);
    const candidates = byDirectory.get(directory) ?? [];
    candidates.push(config);
    candidates.sort((left, right) =>
      configRank(left) - configRank(right) || (left < right ? -1 : left > right ? 1 : 0),
    );
    byDirectory.set(directory, candidates);
  }

  const groups = new Map();
  for (const source of sources) {
    let directory = posix.dirname(source);
    let selected = null;
    for (;;) {
      const candidates = byDirectory.get(directory);
      if (candidates?.length) {
        selected = candidates[0];
        break;
      }
      if (directory === ".") break;
      directory = posix.dirname(directory);
    }

    const grouped = groups.get(selected) ?? [];
    grouped.push(source);
    groups.set(selected, grouped);
  }

  if ([...groups.keys()].filter(Boolean).length > LIMITS.maxConfigs) {
    throw new Error(`TypeScript config limit exceeded (${LIMITS.maxConfigs})`);
  }
  return groups;
}

function extendsValues(config) {
  if (typeof config.extends === "string") return [config.extends];
  if (Array.isArray(config.extends) && config.extends.every((value) => typeof value === "string")) {
    return config.extends;
  }
  if (config.extends === undefined) return [];
  throw new Error("extends must be a string or an array of strings");
}

function resolveExtendedConfig(currentPath, requestedPath, allFiles) {
  if (requestedPath.startsWith("/")) {
    throw new Error(`extended config escapes snapshot: ${requestedPath}`);
  }

  const base = posix.normalize(posix.join(posix.dirname(currentPath), requestedPath));
  if (base === ".." || base.startsWith("../")) {
    throw new Error(`extended config escapes snapshot: ${requestedPath}`);
  }

  const candidates = [base, `${base}.json`, posix.join(base, "tsconfig.json")];
  const match = candidates.find((candidate) => allFiles.has(candidate));
  if (match) return match;

  if (!requestedPath.startsWith(".")) {
    throw new Error(`package-based extended config is unavailable: ${requestedPath}`);
  }
  throw new Error(`extended config is missing: ${requestedPath}`);
}

function readConfig(root, configPath) {
  const errors = [];
  const config = parse(readFileSync(absoluteSnapshotPath(root, configPath), "utf8"), errors, {
    allowTrailingComma: true,
  });
  if (errors.length || !config || typeof config !== "object" || Array.isArray(config)) {
    throw new Error(`malformed JSONC at ${configPath}`);
  }
  return config;
}

function validateCompilerOptions(options) {
  if (!options || typeof options !== "object" || Array.isArray(options)) {
    throw new Error("compilerOptions must be an object");
  }
  if (options.baseUrl !== undefined && typeof options.baseUrl !== "string") {
    throw new Error("baseUrl must be a string");
  }
  if (options.paths !== undefined) {
    if (!options.paths || typeof options.paths !== "object" || Array.isArray(options.paths)) {
      throw new Error("paths must be an object");
    }
    for (const [pattern, targets] of Object.entries(options.paths)) {
      if (!Array.isArray(targets) || targets.length === 0 || targets.some((target) => typeof target !== "string")) {
        throw new Error(`paths mapping must contain string targets: ${pattern}`);
      }
    }
  }
  for (const option of ["rootDirs", "moduleSuffixes"]) {
    if (options[option] !== undefined
      && (!Array.isArray(options[option]) || options[option].some((value) => typeof value !== "string"))) {
      throw new Error(`${option} must be an array of strings`);
    }
  }
  if (options.moduleSuffixes?.some((suffix) => /[/\\]/u.test(suffix))) {
    throw new Error("moduleSuffixes cannot contain path separators");
  }
}

function validateConfig(root, configPath, allFiles, visiting) {
  if (visiting.has(configPath)) throw new Error(`extended config cycle at ${configPath}`);
  visiting.add(configPath);
  const config = readConfig(root, configPath);
  validateCompilerOptions(config.compilerOptions ?? {});
  let options = {};
  for (const extended of extendsValues(config)) {
    options = { ...options, ...validateConfig(root, resolveExtendedConfig(configPath, extended, allFiles), allFiles, visiting) };
  }
  const own = config.compilerOptions ?? {};
  const configDir = dirname(absoluteSnapshotPath(root, configPath));
  if (own.baseUrl !== undefined) {
    const baseUrl = resolve(configDir, own.baseUrl);
    absoluteSnapshotPath(root, baseUrl);
    options.baseUrl = baseUrl;
  }
  if (own.paths !== undefined) options.pathsDir = configDir;
  const rootDirs = own.rootDirs?.map((directory) => {
    const absolute = resolve(configDir, directory);
    absoluteSnapshotPath(root, absolute);
    return absolute;
  }) ?? options.rootDirs;
  const effectivePaths = own.paths ?? options.paths ?? {};
  for (const targets of Object.values(effectivePaths)) {
    for (const target of targets) {
      const candidate = resolve(options.baseUrl ?? configDir, target.replaceAll("${configDir}", options.pathsDir ?? configDir).replaceAll("*", "segment"));
      absoluteSnapshotPath(root, candidate);
    }
  }
  visiting.delete(configPath);
  const merged = { ...options, ...own };
  if (options.baseUrl !== undefined) merged.baseUrl = options.baseUrl;
  if (rootDirs !== undefined) merged.rootDirs = rootDirs;
  return merged;
}

export function loadResolutionConfig(root, configPath, allFiles) {
  try {
    const options = validateConfig(root, configPath, allFiles, new Set());
    return {
      gap: null,
      configPath: absoluteSnapshotPath(root, configPath),
      paths: options.paths ?? {},
      rootDirs: options.rootDirs,
      moduleSuffixes: options.moduleSuffixes,
    };
  } catch (error) {
    return {
      gap: { path: configPath, reason: `invalid TypeScript config: ${error.message}` },
      configPath: null,
    };
  }
}
