import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { selectCliDistributionTargets } from "./select-cli-distribution-targets.mjs";

const configuration = JSON.parse(
  readFileSync(new URL("../../cli/distribution/targets.json", import.meta.url), "utf8"),
);

test("releases build every configured target with the pinned Node runtime", () => {
  const plan = selectCliDistributionTargets(configuration, "release");

  assert.deepEqual(
    plan.include.map(({ target }) => target),
    configuration.targets.map(({ triple }) => triple),
  );
  assert.equal(plan.include.filter(({ smoke }) => smoke).length, 4);
  assert.ok(plan.include.every(({ artifact }) => artifact.endsWith(".tar.gz")));
  assert.match(configuration.node_version, /^\d+\.\d+\.\d+$/);
  assert.ok(plan.include.every(({ node_version, node_platform, node_sha256 }) =>
    node_version === configuration.node_version
    && /^(linux|darwin|win)-(x64|arm64)$/.test(node_platform)
    && /^[a-f0-9]{64}$/.test(node_sha256)));
});

test("pull requests build only the native targets the Linux checks job does not cover", () => {
  const plan = selectCliDistributionTargets(configuration, "pull-request");

  assert.deepEqual(
    plan.include.map(({ target }) => target),
    ["aarch64-apple-darwin", "x86_64-pc-windows-msvc"],
  );
  assert.ok(plan.include.every(({ smoke }) => smoke));
  assert.ok(plan.include.every(({ runner }) => runner.startsWith("blacksmith-")));
});

test("configurations without targets, a pinned Node version, or a known mode fail", () => {
  assert.throws(() => selectCliDistributionTargets({ targets: [] }, "release"), /non-empty targets/);
  assert.throws(
    () => selectCliDistributionTargets({ targets: configuration.targets }, "release"),
    /node_version/,
  );
  assert.throws(() => selectCliDistributionTargets(configuration, "nightly"), /Unknown CLI distribution mode/);
  assert.throws(() => selectCliDistributionTargets(configuration), /Unknown CLI distribution mode/);
});
