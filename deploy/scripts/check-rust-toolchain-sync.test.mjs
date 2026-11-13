import assert from "node:assert/strict";
import test from "node:test";

import {
  readToolchainFiles,
  validateRustToolchainSync,
} from "./check-rust-toolchain-sync.mjs";

const liveFiles = readToolchainFiles();

test("a mismatched Docker Rust base image fails with the replica name and versions", () => {
  const files = { ...liveFiles };
  const expectedVersion = files["rust-toolchain.toml"].match(
    /channel\s*=\s*"([^"]+)"/,
  )[1];
  files["media-worker/Dockerfile"] = files["media-worker/Dockerfile"].replace(
    `rust:${expectedVersion}-trixie`,
    "rust:1.97.0-trixie",
  );

  assert.deepEqual(validateRustToolchainSync(files), [
    `media-worker/Dockerfile: Rust base image must match Rust ${expectedVersion}; found 1.97.0`,
  ]);
});

test("a missing replica fails instead of passing silently", () => {
  const files = { ...liveFiles };
  delete files["media-worker/Dockerfile"];

  assert.deepEqual(validateRustToolchainSync(files), ["media-worker/Dockerfile: file is missing"]);
});
