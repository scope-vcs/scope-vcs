import { ANALYZER_VERSION, LIMITS } from "./constants.mjs";
import { groupSourcesByConfig, loadResolutionConfig } from "./config.mjs";
import { cruiseGroup } from "./cruiser.mjs";
import { inventorySnapshot, SnapshotError } from "./snapshot.mjs";
import { scanSources } from "./source-scan.mjs";

function baseResult() {
  return {
    analyzer_version: ANALYZER_VERSION,
    analyzed_files: [],
    unsupported_files: [],
    edges: [],
    gaps: [],
  };
}

function uniqueSorted(items, keyOf) {
  return [...new Map(items.map((item) => [keyOf(item), item])).values()].sort((left, right) => {
    const leftKey = keyOf(left);
    const rightKey = keyOf(right);
    return leftKey < rightKey ? -1 : leftKey > rightKey ? 1 : 0;
  });
}

function boundedGaps(gaps) {
  const unique = uniqueSorted(gaps, ({ path, reason }) => `${path}\0${reason}`);
  if (unique.length <= LIMITS.maxGaps) return unique;
  return [
    ...unique.slice(0, LIMITS.maxGaps - 1),
    { path: ".", reason: `coverage gap limit exceeded (${LIMITS.maxGaps})` },
  ];
}

export async function analyzeSnapshot(snapshotPath) {
  let inventory;
  try {
    inventory = await inventorySnapshot(snapshotPath);
  } catch (error) {
    if (!(error instanceof SnapshotError)) throw error;
    return { ...baseResult(), gaps: [{ path: ".", reason: error.message }] };
  }

  const scan = await scanSources(inventory.root, inventory.sources);
  let groups;
  try {
    groups = groupSourcesByConfig(scan.cruisableFiles, inventory.configs);
  } catch (error) {
    return {
      ...baseResult(),
      analyzed_files: scan.analyzedFiles,
      unsupported_files: inventory.unsupportedFiles,
      gaps: boundedGaps([...scan.gaps, { path: ".", reason: error.message }]),
    };
  }

  const edges = [];
  const gaps = [...scan.gaps];
  for (const [configPath, sources] of groups) {
    const config = configPath
      ? loadResolutionConfig(inventory.root, configPath, inventory.allFiles)
      : { gap: null, transpileOptions: undefined };
    if (config.gap) gaps.push(config.gap);

    try {
      const groupResult = await cruiseGroup({
        allFiles: inventory.allFiles,
        configPath: config.gap ? null : configPath,
        referencesBySource: scan.referencesBySource,
        root: inventory.root,
        sources,
        transpileOptions: config.transpileOptions,
      });
      edges.push(...groupResult.edges);
      gaps.push(...groupResult.gaps);
    } catch (error) {
      gaps.push({
        path: configPath ?? ".",
        reason: `dependency analysis failed: ${error.message}`,
      });
    }
  }

  const uniqueEdges = uniqueSorted(
    edges,
    ({ source_path, target_path, kind }) => `${source_path}\0${target_path}\0${kind}`,
  );
  if (uniqueEdges.length > LIMITS.maxEdges) {
    gaps.push({ path: ".", reason: `dependency edge limit exceeded (${LIMITS.maxEdges})` });
    uniqueEdges.length = 0;
  }

  return {
    analyzer_version: ANALYZER_VERSION,
    analyzed_files: scan.analyzedFiles,
    unsupported_files: inventory.unsupportedFiles,
    edges: uniqueEdges,
    gaps: boundedGaps(gaps),
  };
}
