import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import test from "node:test";

import {
  classifyChanges,
  includeMigrationParticipants,
  planFromDeploymentProgress,
} from "./plan-production-deployment.mjs";

const manifest = JSON.parse(readFileSync(new URL("../deployment-services.json", import.meta.url), "utf8"));

function repositoryJson(path) {
  return JSON.parse(readFileSync(new URL(`../../${path}`, import.meta.url), "utf8"));
}

function deploymentSelection(overrides = {}) {
  return {
    "checks-image": false,
    cache: false,
    "run-worker": false,
    "media-worker": false,
    "git-router": false,
    "media-api": false,
    api: false,
    web: false,
    "cli-downloads": false,
    "cli-distribution": false,
    ...overrides,
  };
}

test("changes select the required deployment lanes", () => {
  const allLanes = {
    "checks-image": true,
    cache: true,
    "run-worker": true,
    "media-worker": true,
    "git-router": true,
    "media-api": true,
    api: true,
    web: true,
    "cli-downloads": true,
    "cli-distribution": true,
  };
  const cases = [
    ["license changes rebuild all distributions", ["LICENSE"], allLanes],
    ["attribution changes rebuild all distributions", ["NOTICE"], allLanes],
    ["dependency notices rebuild all distributions", ["legal/third-party-rust.txt"], allLanes],
    ["documentation-only changes do not deploy", ["docs/cache.md"], {}],
    ["cache service changes run backend only", ["cache-service/src/main.rs"], { cache: true }],
    [
      "runner changes publish the checks image and nothing else",
      ["runner-runtime/src/main.rs"],
      { "checks-image": true },
    ],
    [
      "toolchain changes publish the checks image and rebuild Rust services",
      ["rust-toolchain.toml"],
      {
        "checks-image": true,
        cache: true,
        "run-worker": true,
        "media-worker": true,
        "git-router": true,
        "media-api": true,
        api: true,
        "cli-downloads": true,
        "cli-distribution": true,
      },
    ],
    ["web-only changes deploy only web", ["web/src/routes/+page.svelte"], { web: true }],
    [
      "API implementation changes validate CLI without rebuilding distribution targets",
      ["api/src/main.rs"],
      { api: true, web: true, "cli-downloads": true },
    ],
    ["router changes deploy the Git router", ["repo-router/src/main.rs"], { "git-router": true }],
    [
      "backend runtime image selects every backend service",
      ["deploy/railway/prebuilt.Dockerfile"],
      { cache: true, "run-worker": true, "git-router": true, api: true },
    ],
    [
      "dependency analyzer changes rebuild the worker and CLI bundle",
      ["dependency-analyzer/analyze.mjs"],
      { "run-worker": true, "cli-downloads": true, "cli-distribution": true },
    ],
    [
      "worker runtime image selects only the worker",
      ["deploy/railway/worker.Dockerfile"],
      { "run-worker": true },
    ],
    [
      "web runtime image selects the web service",
      ["deploy/railway/web.Dockerfile"],
      { web: true },
    ],
    [
      "CLI prebuilt config selects CLI without rebuilding distributions",
      ["deploy/railway/prebuilt-cli.railpack.json"],
      { "cli-downloads": true },
    ],
    [
      "CLI tests validate CLI without rebuilding distribution targets",
      ["cli/tests/request.rs"],
      { "cli-downloads": true },
    ],
    [
      "CLI source changes rebuild distribution targets",
      ["cli/src/request.rs"],
      { "cli-downloads": true, "cli-distribution": true },
    ],
    [
      "distribution config changes rebuild distribution targets",
      ["cli/distribution/targets.json"],
      { "cli-downloads": true, "cli-distribution": true },
    ],
    [
      "unrelated shared crates retain broad CLI validation without rebuilding targets",
      ["crates/scope-cache-contract/src/lib.rs"],
      {
        "checks-image": true,
        cache: true,
        "run-worker": true,
        "media-worker": true,
        "git-router": true,
        "media-api": true,
        api: true,
        web: true,
        "cli-downloads": true,
      },
    ],
    [
      "shared workspace changes preserve the previous conservative scope",
      ["crates/scope-domain/src/lib.rs"],
      allLanes,
    ],
    [
      "orchestration workflow changes exercise every lane",
      [".github/workflows/validate.yml"],
      allLanes,
    ],
    [
      "component workflow changes exercise only their lane",
      [".github/workflows/scope-cli-build.yml"],
      { "cli-downloads": true, "cli-distribution": true },
    ],
    [
      "backend workflow changes rebuild every backend service",
      [".github/workflows/deploy-backend.yml"],
      { cache: true, "run-worker": true, "media-worker": true, "git-router": true, "media-api": true, api: true },
    ],
    [
      "artifact-shaping scripts select the lanes they package",
      [".github/scripts/select-cli-distribution-targets.mjs"],
      { "cli-downloads": true, "cli-distribution": true },
    ],
    [
      "release preparation scripts rebuild every prepared image",
      [".github/scripts/prepare-railway-artifact.sh"],
      { cache: true, "run-worker": true, "git-router": true, "media-api": true, api: true, web: true },
    ],
    [
      "the production deploy wrapper reruns every service it activates",
      [".github/scripts/deploy-monitored-railway.sh"],
      { cache: true, "run-worker": true, "media-worker": true, "git-router": true, "media-api": true, api: true, web: true },
    ],
    [
      "check entrypoints select the lane they gate",
      ["dev/checks/web"],
      { web: true },
    ],
    [
      "the contract check belongs to the API lane",
      ["dev/checks/contract"],
      { api: true },
    ],
    [
      "local stack tooling reruns the integration consumers",
      ["dev/scope-dev"],
      { "media-worker": true, "media-api": true, web: true, "cli-downloads": true },
    ],
    [
      "deploy-time and operations scripts are gated by the always-on operations checks",
      [".github/scripts/deploy-backend-railway.sh", ".github/scripts/staging-baseline.sh", "bench/railway-load.mjs", "dev/checks/ops", "dev/analytics/reports.mjs"],
      {},
    ],
  ];

  for (const [name, paths, lanes] of cases) {
    assert.deepEqual(classifyChanges(manifest, paths), deploymentSelection(lanes), name);
  }
});

test("manual component and all scopes are explicit", () => {
  assert.deepEqual(classifyChanges(manifest, [], "web"), deploymentSelection({ web: true }));
  assert.deepEqual(
    classifyChanges(manifest, [], "cli-downloads"),
    deploymentSelection({ "cli-downloads": true, "cli-distribution": true }),
  );
  assert.ok(Object.values(classifyChanges(manifest, [], "all")).every(Boolean));
  assert.throws(
    () => classifyChanges(manifest, [], "cli-distribution"),
    /Unknown deployment scope/,
  );
  assert.throws(() => classifyChanges(manifest, [], "database"), /Unknown deployment scope/);
});

test("planner emits backend and CLI distribution workflow outputs", () => {
  for (const [scope, expected] of [
    ["cli-downloads", ["cli=true", "cli_distribution=true", "backend_selected=false"]],
    ["api", ["api=true", "backend_selected=true"]],
  ]) {
    const output = execFileSync(process.execPath, [
      fileURLToPath(new URL("./plan-production-deployment.mjs", import.meta.url)),
      "--manifest",
      fileURLToPath(new URL("../deployment-services.json", import.meta.url)),
      "--scope",
      scope,
    ], {
      encoding: "utf8",
      env: { ...process.env, GITHUB_OUTPUT: "", GITHUB_STEP_SUMMARY: "" },
    });
    for (const line of expected) assert.ok(output.split("\n").includes(line), line);
  }
});

test("an unseeded production ledger deploys every component", () => {
  assert.ok(Object.values(planFromDeploymentProgress(manifest, {})).every(Boolean));
});

test("skipped components remain selected across a later backend-only change", () => {
  const selection = planFromDeploymentProgress(manifest, {
    "checks-image": [],
    cache: ["cache-service/src/main.rs"],
    "run-worker": [],
    "media-worker": [],
    "git-router": [],
    "media-api": [],
    api: [],
    // Web last succeeded before commit A. Its component-specific range still includes A's
    // web change when commit B changes only the cache service after A's web job was skipped.
    web: ["web/src/routes/+page.svelte", "cache-service/src/main.rs"],
    "cli-downloads": [],
  });

  assert.deepEqual(selection, {
    "checks-image": false,
    cache: true,
    "run-worker": false,
    "media-worker": false,
    "git-router": false,
    "media-api": false,
    api: false,
    web: true,
    "cli-downloads": false,
    "cli-distribution": false,
  });
});

test("CLI deployment progress selects distribution builds only for binary inputs", () => {
  const broadOnly = planFromDeploymentProgress(manifest, {
    "checks-image": [],
    cache: [],
    "run-worker": [],
    "media-worker": [],
    "git-router": [],
    "media-api": [],
    api: [],
    web: [],
    "cli-downloads": ["api/src/main.rs"],
  });
  const binaryChange = planFromDeploymentProgress(manifest, {
    "checks-image": [],
    cache: [],
    "run-worker": [],
    "media-worker": [],
    "git-router": [],
    "media-api": [],
    api: [],
    web: [],
    "cli-downloads": ["crates/scope-api-contract/src/lib.rs"],
  });

  assert.deepEqual(broadOnly, deploymentSelection({ "cli-downloads": true }));
  assert.deepEqual(
    binaryChange,
    deploymentSelection({ "cli-downloads": true, "cli-distribution": true }),
  );
});

test("manual scopes ignore pending production components", () => {
  assert.deepEqual(planFromDeploymentProgress(manifest, {}, "web"), {
    "checks-image": false,
    cache: false,
    "run-worker": false,
    "media-worker": false,
    "git-router": false,
    "media-api": false,
    api: false,
    web: true,
    "cli-downloads": false,
    "cli-distribution": false,
  });
});

test("deployment manifest is a single coherent production graph", () => {
  const order = ["cache", "run-worker", "git-router", "media-api", "media-worker", "api", "web", "cli-downloads"];
  const serviceIds = order.map((service) => manifest.services[service].id).filter(Boolean);

  assert.equal(manifest.deploymentAuthority, "github-actions");
  assert.equal(manifest.source.nativeAutodeploy, false);
  assert.equal(new Set(serviceIds).size, serviceIds.length);
  assert.match(manifest.services["media-api"].id, /^[0-9a-f-]{36}$/);
  assert.match(manifest.services["media-worker"].id, /^[0-9a-f-]{36}$/);
  assert.match(manifest.mediaResources.bucket.id, /^[0-9a-f-]{36}$/);
  const mediaDomains = ["production", "staging"].map((environment) => (
    manifest.mediaResources[environment].gatewayDomain
  ));
  for (const domain of mediaDomains) assert.match(domain, /^[a-z0-9-]+\.up\.railway\.app$/);
  assert.equal(new Set(mediaDomains).size, mediaDomains.length);
  for (const [service, configuration] of Object.entries(manifest.services)) {
    for (const dependency of configuration.dependsOn) {
      assert.ok(order.indexOf(dependency) < order.indexOf(service));
    }
  }
});

test("service config does not override Railway scaling or restart defaults", () => {
  const configs = {
    "api/railway.json": "/readyz",
    "worker/railway.json": "/readyz",
    "cache-service/railway.json": "/readyz",
    "repo-router/railway.json": "/readyz",
    "media-service/railway.json": "/readyz",
    "cli/railway.json": "/readyz",
    "web/railway.json": "/readyz",
  };
  for (const [path, healthcheckPath] of Object.entries(configs)) {
    const { deploy } = repositoryJson(path);

    assert.equal(deploy.healthcheckPath, healthcheckPath);
    assert.equal(deploy.healthcheckTimeout, 60);
    assert.equal(deploy.multiRegionConfig, undefined);
    assert.equal(deploy.restartPolicyType, undefined);
    assert.equal(deploy.restartPolicyMaxRetries, undefined);
  }
});

test("shared process lifecycle changes validate and rebuild the checks image", () => {
  const selected = classifyChanges(manifest, ["crates/scope-git-process/src/lifecycle.rs"]);
  assert.equal(selected["checks-image"], true);
});

test("checks images pass the container lifecycle gate before publication", () => {
  const checks = readFileSync(new URL("../workflows/scope-checks-image.yml", import.meta.url), "utf8");
  const candidate = checks.slice(checks.indexOf("  validate:"), checks.indexOf("  build:"));
  const build = checks.slice(checks.indexOf("  build:"));
  for (const lane of [candidate, build]) {
    assert.match(lane, /push: false\n\s+load: true/);
    assert.match(lane, /run: dev\/checks\/runner-runtime-container --image /);
  }
  const verify = build.indexOf("- name: Verify runtime process lifecycle");
  const publish = build.indexOf("- name: Publish verified image");
  const promote = build.indexOf("- name: Publish raw and SOCI v2 variants");
  assert(verify >= 0 && publish > verify && promote > publish);
  assert.match(build.slice(publish, promote), /docker push "\$tag"/);
});

test("migration changes promote every application participant but leave checks images independent", () => {
  for (const apiChanges of [null, undefined, ["crates/scope-postgres/src/migrations/999_next.rs"]]) {
    const selected = includeMigrationParticipants(deploymentSelection({ api: true }), apiChanges);
    assert.equal(selected["checks-image"], false);
    for (const [component, value] of Object.entries(selected)) if (component !== "checks-image") assert.equal(value, true, component);
  }
  const apiOnly = deploymentSelection({ api: true });
  assert.deepEqual(includeMigrationParticipants(apiOnly, ["api/src/main.rs"]), apiOnly);
  const webOnly = deploymentSelection({ web: true });
  assert.deepEqual(includeMigrationParticipants(webOnly, null), webOnly);
  assert.equal(planFromDeploymentProgress(manifest, { api: ["crates/scope-postgres/src/migrations/999_next.rs"] }, "run-worker").web, true);
  assert.deepEqual(planFromDeploymentProgress(manifest, { api: [] }, "run-worker"), deploymentSelection({ "run-worker": true }));
});
