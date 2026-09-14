import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { selectCliDistributionTargets } from "./select-cli-distribution-targets.mjs";

const configuration = JSON.parse(
  readFileSync(new URL("../../cli/distribution/targets.json", import.meta.url), "utf8"),
);

test("every configured target builds a bundle with the pinned Node runtime", () => {
  const plan = selectCliDistributionTargets(configuration);

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

test("configurations without targets or a pinned Node version fail", () => {
  assert.throws(() => selectCliDistributionTargets({ targets: [] }), /non-empty targets/);
  assert.throws(
    () => selectCliDistributionTargets({ targets: configuration.targets }),
    /node_version/,
  );
});
