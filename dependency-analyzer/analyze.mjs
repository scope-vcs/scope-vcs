#!/usr/bin/env node

import { ANALYZER_VERSION } from "./src/constants.mjs";
import { analyzeSnapshot } from "./src/analyzer.mjs";
import { serializeOutput } from "./src/output.mjs";

async function main(arguments_) {
  if (arguments_.length === 1 && arguments_[0] === "--version") {
    process.stdout.write(`${ANALYZER_VERSION}\n`);
    return;
  }
  if (arguments_.length !== 1) {
    process.stderr.write("usage: node dependency-analyzer/analyze.mjs <snapshot-dir>\n");
    process.exitCode = 64;
    return;
  }

  const result = await analyzeSnapshot(arguments_[0]);
  process.stdout.write(serializeOutput(result));
}

main(process.argv.slice(2)).catch((error) => {
  process.stderr.write(`dependency analysis failed: ${error.message}\n`);
  process.exitCode = 1;
});
