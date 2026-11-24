import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

import {
  selectCliDistributionTargets,
  selectNativeArtifactChecks,
  workflowOutputs,
} from "./select-cli-distribution-targets.mjs";

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
  assert.ok(plan.include.every(({ profile }) => profile === "release"));
  assert.equal(
    plan.include.find(({ target }) => target === "x86_64-pc-windows-msvc").binary,
    "target/x86_64-pc-windows-msvc/release/scope.exe",
  );
});

test("pull requests build only the Linux and Windows x64 targets without the shipped LTO profile", () => {
  const plan = selectCliDistributionTargets(configuration, "pull-request");

  assert.deepEqual(
    plan.include.map(({ target }) => target),
    ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "x86_64-pc-windows-msvc"],
  );
  assert.ok(plan.include.every(({ smoke, native_runner }) => smoke || native_runner));
  assert.deepEqual(plan.include.map(({ binary }) => binary), [
    "target/x86_64-unknown-linux-gnu/release-check/scope",
    "target/aarch64-unknown-linux-gnu/release-check/scope",
    "target/x86_64-pc-windows-msvc/release-check/scope.exe",
  ]);
});

test("every release target that skips native checks on its build host runs its artifact on a native host", () => {
  const plan = selectCliDistributionTargets(configuration, "release");
  const checks = selectNativeArtifactChecks(plan);

  assert.deepEqual(
    checks.include.map(({ target, native_runner }) => [target, native_runner]),
    [["aarch64-unknown-linux-gnu", "ubuntu-24.04-arm"]],
  );
  const unexecuted = plan.include.filter(({ smoke, native_runner }) => !smoke && !native_runner);
  assert.deepEqual(unexecuted.map(({ target }) => target), ["aarch64-pc-windows-msvc"]);
});

test("workflow outputs leave native checks empty when no selected target needs them", () => {
  const release = workflowOutputs(configuration, "release").split("\n");
  assert.match(release[0], /^matrix=\{"include":\[/);
  assert.match(release[1], /^native_checks=\{"include":\[\{.*"native_runner":"ubuntu-24\.04-arm"/);

  const pullRequest = workflowOutputs(configuration, "pull-request").split("\n");
  assert.match(pullRequest[1], /^native_checks={"include"/);
});

test("configurations without targets or a known mode fail", () => {
  assert.throws(() => selectCliDistributionTargets({ targets: [] }, "release"), /non-empty targets/);
  assert.throws(() => selectCliDistributionTargets(configuration, "nightly"), /Unknown CLI distribution mode/);
  assert.throws(() => selectCliDistributionTargets(configuration), /Unknown CLI distribution mode/);
});
