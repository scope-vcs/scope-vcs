import assert from "node:assert/strict";
import test from "node:test";
import { ANALYZER_VERSION } from "../src/constants.mjs";
import { MAX_OUTPUT_BYTES, serializeOutput } from "../src/output.mjs";

test("oversized results become a bounded coverage gap instead of failed worker output", () => {
  const result = {
    analyzer_version: ANALYZER_VERSION,
    analyzed_files: Array.from({ length: 2_200 }, (_, i) => `${"a/".repeat(2_000)}${i}.ts`),
    unsupported_files: [],
    edges: [],
    gaps: [],
  };
  assert.ok(Buffer.byteLength(JSON.stringify(result)) > MAX_OUTPUT_BYTES);
  const serialized = serializeOutput(result);
  assert.ok(Buffer.byteLength(serialized) <= MAX_OUTPUT_BYTES);
  const output = JSON.parse(serialized);
  assert.equal(output.analyzer_version, ANALYZER_VERSION);
  assert.deepEqual(output.analyzed_files, []);
  assert.deepEqual(output.edges, []);
  assert.equal(output.gaps[0].path, ".");
  assert.match(output.gaps[0].reason, /analysis is incomplete/);
});

test("output budget counts UTF-8 bytes and the trailing newline", () => {
  const result = { value: "" };
  const overhead = Buffer.byteLength(serializeOutput(result));
  result.value = "é".repeat(Math.floor((MAX_OUTPUT_BYTES - overhead) / 2));
  assert.deepEqual(JSON.parse(serializeOutput(result)), result);
  result.value += "é";
  assert.equal(JSON.parse(serializeOutput(result)).gaps[0].path, ".");
});
