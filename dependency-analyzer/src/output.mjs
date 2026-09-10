import { ANALYZER_VERSION } from "./constants.mjs";

// Includes the newline and matches the worker's stdout budget.
export const MAX_OUTPUT_BYTES = 8 * 1024 * 1024;

export function serializeOutput(result) {
  const serialized = `${JSON.stringify(result)}\n`;
  if (Buffer.byteLength(serialized) <= MAX_OUTPUT_BYTES) return serialized;
  return `${JSON.stringify({
    analyzer_version: ANALYZER_VERSION,
    analyzed_files: [],
    unsupported_files: [],
    edges: [],
    gaps: [{ path: ".", reason: "Dependency results exceed the 8 MiB limit; analysis is incomplete." }],
  })}\n`;
}
