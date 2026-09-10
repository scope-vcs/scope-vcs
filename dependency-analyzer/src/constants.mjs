export const ANALYZER_VERSION = "dependency-cruiser@18.2.0+scope-1";

export const SUPPORTED_SOURCE_EXTENSIONS = new Set([
  ".cjs",
  ".cts",
  ".js",
  ".jsx",
  ".mjs",
  ".mts",
  ".ts",
  ".tsx",
]);

export const UNSUPPORTED_SOURCE_EXTENSIONS = new Set([
  ".c",
  ".cc",
  ".clj",
  ".cljs",
  ".coffee",
  ".cpp",
  ".cs",
  ".dart",
  ".erl",
  ".ex",
  ".exs",
  ".fs",
  ".fsx",
  ".go",
  ".h",
  ".hpp",
  ".hrl",
  ".java",
  ".kt",
  ".kts",
  ".lua",
  ".php",
  ".pl",
  ".py",
  ".rb",
  ".rs",
  ".scala",
  ".svelte",
  ".swift",
  ".vue",
  ".astro",
]);

export const LIMITS = Object.freeze({
  maxConfigs: 64,
  maxEdges: 25_000,
  maxFiles: 20_000,
  maxGaps: 10_000,
  maxPathBytes: 4_096,
  maxSourceBytes: 2 * 1024 * 1024,
  maxSources: 20_000,
  maxTotalSourceBytes: 64 * 1024 * 1024,
  maxUnsupportedSources: 5_000,
});

export const IGNORED_DIRECTORY_NAMES = new Set([
  ".git",
  ".hg",
  ".svn",
  "node_modules",
]);
