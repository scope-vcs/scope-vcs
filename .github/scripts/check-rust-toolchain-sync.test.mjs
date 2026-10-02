import assert from "node:assert/strict";
import test from "node:test";

import {
  readToolchainFiles,
  validateRustToolchainSync,
} from "./check-rust-toolchain-sync.mjs";

const liveFiles = readToolchainFiles();

test("a mismatched runner base image fails with the replica name and versions", () => {
  const files = { ...liveFiles };
  const expectedVersion = files["rust-toolchain.toml"].match(
    /channel\s*=\s*"([^"]+)"/,
  )[1];
  files["runner-runtime/Dockerfile"] = files["runner-runtime/Dockerfile"].replace(
    `rust:${expectedVersion}-slim-bookworm`,
    "rust:1.97.0-slim-bookworm",
  );

  assert.deepEqual(validateRustToolchainSync(files), [
    `runner-runtime/Dockerfile: Rust base image must match Rust ${expectedVersion}; found 1.97.0`,
  ]);
});

test("a missing replica fails instead of passing silently", () => {
  const files = { ...liveFiles };
  delete files["runner-runtime/Dockerfile"];

  assert.deepEqual(validateRustToolchainSync(files), ["runner-runtime/Dockerfile: file is missing"]);
});
