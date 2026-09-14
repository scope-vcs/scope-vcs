import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { selectCliDistributionTargets } from "./select-cli-distribution-targets.mjs";

const configuration = JSON.parse(
  readFileSync(new URL("../../cli/distribution/targets.json", import.meta.url), "utf8"),
);

test("pull requests build every supported distribution target", () => {
  const plan = selectCliDistributionTargets(configuration, "pull-request");

  assert.deepEqual(
    plan.include.map(({ target }) => target),
    configuration.targets.map(({ triple }) => triple),
  );
  assert.equal(plan.include.filter(({ smoke }) => smoke).length, 4);
});

test("release runs retain every configured native target", () => {
  const plan = selectCliDistributionTargets(configuration, "release");

  assert.deepEqual(
    plan.include.map(({ target }) => target),
    configuration.targets.map(({ triple }) => triple),
  );
  assert.ok(plan.include.every(({ artifact }) => artifact.endsWith(".tar.gz")));
  assert.ok(plan.include.every(({ node_archive, node_sha256, node_directory, node_executable }) =>
    node_archive.startsWith("node-v24.21.0-")
    && /^[a-f0-9]{64}$/.test(node_sha256)
    && node_directory.startsWith("node-v24.21.0-")
    && /^(bin\/node|node\.exe)$/.test(node_executable)));
});

test("unknown modes fail instead of silently dropping release targets", () => {
  assert.throws(
    () => selectCliDistributionTargets(configuration, "nightly"),
    /Unknown CLI distribution mode/,
  );
});
