import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import { selectCliDistributionTargets } from "./select-cli-distribution-targets.mjs";

const configuration = JSON.parse(
  readFileSync(new URL("../../cli/distribution/targets.json", import.meta.url), "utf8"),
);

test("releases build every configured target", () => {
  const plan = selectCliDistributionTargets(configuration, "release");

  assert.deepEqual(
    plan.include.map(({ target }) => target),
    configuration.targets.map(({ triple }) => triple),
  );
  assert.equal(plan.include.filter(({ smoke }) => smoke).length, 4);
  assert.ok(plan.include.every(({ artifact }) => artifact.endsWith(".tar.gz")));
});

test("pull requests build only the Linux and Windows x64 targets", () => {
  const plan = selectCliDistributionTargets(configuration, "pull-request");

  assert.deepEqual(
    plan.include.map(({ target }) => target),
    ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc"],
  );
  assert.ok(plan.include.every(({ smoke }) => smoke));
});

test("configurations without targets or a known mode fail", () => {
  assert.throws(() => selectCliDistributionTargets({ targets: [] }, "release"), /non-empty targets/);
  assert.throws(() => selectCliDistributionTargets(configuration, "nightly"), /Unknown CLI distribution mode/);
  assert.throws(() => selectCliDistributionTargets(configuration), /Unknown CLI distribution mode/);
});
