import { dirname, posix } from "node:path";

import ts from "typescript";

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

function diagnosticReason(diagnostic) {
  return ts.flattenDiagnosticMessageText(diagnostic.messageText, " ").slice(0, 400);
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
  const result = ts.readConfigFile(absoluteSnapshotPath(root, configPath), ts.sys.readFile);
  if (result.error) throw new Error(diagnosticReason(result.error));
  return result.config;
}

function loadCompilerOptions(root, configPath, allFiles, visiting) {
  if (visiting.has(configPath)) throw new Error(`extended config cycle at ${configPath}`);
  visiting.add(configPath);

  const config = readConfig(root, configPath);
  let options = {};
  for (const extended of extendsValues(config)) {
    const extendedPath = resolveExtendedConfig(configPath, extended, allFiles);
    options = { ...options, ...loadCompilerOptions(root, extendedPath, allFiles, visiting) };
  }

  const converted = ts.convertCompilerOptionsFromJson(
    config.compilerOptions ?? {},
    dirname(absoluteSnapshotPath(root, configPath)),
    configPath,
  );
  if (converted.errors.length > 0) throw new Error(diagnosticReason(converted.errors[0]));

  if (converted.options.paths && !converted.options.baseUrl && !options.baseUrl) {
    converted.options.baseUrl = dirname(absoluteSnapshotPath(root, configPath));
  }

  visiting.delete(configPath);
  return { ...options, ...converted.options };
}

export function loadResolutionConfig(root, configPath, allFiles) {
  try {
    return {
      gap: null,
      transpileOptions: {
        tsConfig: { options: loadCompilerOptions(root, configPath, allFiles, new Set()) },
      },
    };
  } catch (error) {
    return {
      gap: { path: configPath, reason: `invalid TypeScript config: ${error.message}` },
      transpileOptions: undefined,
    };
  }
}
