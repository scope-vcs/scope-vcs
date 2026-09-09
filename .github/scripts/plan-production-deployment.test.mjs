import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { runInNewContext } from "node:vm";

import {
  classifyChanges,
  planFromDeploymentProgress,
} from "./plan-production-deployment.mjs";

const manifest = JSON.parse(readFileSync(new URL("../deployment-services.json", import.meta.url), "utf8"));
const productionWorkflow = readFileSync(
  new URL("../workflows/scope-production-deploy.yml", import.meta.url),
  "utf8",
);
const backendCiWorkflow = readFileSync(
  new URL("../workflows/scope-api-ci.yml", import.meta.url),
  "utf8",
);
const backendDeployWorkflow = readFileSync(
  new URL("../workflows/scope-api-deploy.yml", import.meta.url),
  "utf8",
);
const cliBuildWorkflow = readFileSync(
  new URL("../workflows/scope-cli-build.yml", import.meta.url),
  "utf8",
);
const integrationCiWorkflow = readFileSync(
  new URL("../workflows/scope-integration-ci.yml", import.meta.url),
  "utf8",
);
const rustChecksWorkflow = readFileSync(
  new URL("../workflows/rust-workspace-checks.yml", import.meta.url),
  "utf8",
);
const cliDeployWorkflow = readFileSync(
  new URL("../workflows/scope-cli-deploy.yml", import.meta.url),
  "utf8",
);
const stagingWorkflow = readFileSync(
  new URL("../workflows/scope-railway-staging.yml", import.meta.url),
  "utf8",
);
const webDeployWorkflow = readFileSync(
  new URL("../workflows/scope-web-deploy.yml", import.meta.url),
  "utf8",
);
const webCiWorkflow = readFileSync(
  new URL("../workflows/scope-web-ci.yml", import.meta.url),
  "utf8",
);

function repositoryJson(path) {
  return JSON.parse(readFileSync(new URL(`../../${path}`, import.meta.url), "utf8"));
}

function deploymentSelection(overrides = {}) {
  return {
    checksImage: false,
    cache: false,
    worker: false,
    mediaWorker: false,
    router: false,
    media: false,
    api: false,
    web: false,
    cli: false,
    cliDistribution: false,
    ...overrides,
  };
}

function productionJobCondition(jobName) {
  const block = productionWorkflow.match(
    new RegExp(`\\n  ${jobName}:\\n([\\s\\S]*?)(?=\\n  [a-z][a-z-]+:|$)`),
  )?.[1];
  assert.ok(block, `${jobName} job is present`);
  const condition = block.match(/\n    if: >-\n([\s\S]*?)(?=\n    [a-z])/i)?.[1];
  assert.ok(condition, `${jobName} has a multiline condition`);
  return condition.trim().replace(/\s+/g, " ");
}

function evaluateProductionCondition(expression, context) {
  const resolve = (path) => path.split(".").reduce((value, key) => value?.[key], context);
  const executable = expression.replace(
    /cancelled\(\)|(?:github|needs|inputs)(?:\.[A-Za-z0-9_-]+)+/g,
    (reference) => JSON.stringify(
      reference === "cancelled()" ? context.cancelled : resolve(reference),
    ),
  );
  const unsupported = executable.replace(
    /true|false|"(?:[^"\\]|\\.)*"|'(?:[^'\\]|\\.)*'|&&|\|\||==|!=|!|\(|\)|\s+/g,
    "",
  );
  assert.equal(unsupported, "", `unsupported workflow expression syntax: ${unsupported}`);
  return Boolean(runInNewContext(executable, Object.create(null), { timeout: 100 }));
}

function productionConditionContext(overrides = {}) {
  const backendSelected = overrides.backendSelected ?? false;
  return {
    cancelled: overrides.cancelled ?? false,
    inputs: {
      recover_cutover_id: overrides.recoveryId ?? "",
    },
    github: {
      event_name: overrides.eventName ?? "schedule",
      ref: overrides.ref ?? "refs/heads/main",
    },
    needs: {
      "media-worker-image": { result: overrides.mediaWorkerImageResult ?? (backendSelected ? "success" : "skipped") },
      "release-preparation": { result: overrides.preparationResult ?? "success" },
      "release-staging-proof": { result: overrides.stagingResult ?? "success" },
      "backend-deploy": { result: overrides.backendResult ?? "skipped" },
      "cli-deploy": {
        result: overrides.cliResult ?? (overrides.cliSelected === false ? "skipped" : "success"),
      },
      plan: {
        outputs: {
          api: backendSelected ? "true" : "false",
          cache: backendSelected ? "true" : "false",
          media: backendSelected ? "true" : "false",
          media_worker: backendSelected ? "true" : "false",
          cli: overrides.cliSelected === false ? "false" : "true",
          router: backendSelected ? "true" : "false",
          web: overrides.webSelected === false ? "false" : "true",
          worker: backendSelected ? "true" : "false",
        },
      },
      "production-validation-gate": {
        result: overrides.validationResult ?? "success",
      },
      "web-deploy": {
        result: overrides.webResult ?? (overrides.webSelected === false ? "skipped" : "success"),
      },
    },
  };
}

test("changes select the required deployment lanes", () => {
  const allLanes = {
    checksImage: true,
    cache: true,
    worker: true,
    mediaWorker: true,
    router: true,
    media: true,
    api: true,
    web: true,
    cli: true,
    cliDistribution: true,
  };
  const cases = [
    ["license changes rebuild all distributions", ["LICENSE"], allLanes],
    ["attribution changes rebuild all distributions", ["NOTICE"], allLanes],
    ["dependency notices rebuild all distributions", ["legal/third-party-rust.txt"], allLanes],
    ["documentation-only changes do not deploy", ["docs/cache.md"], {}],
    ["cache service changes run backend only", ["cache-service/src/main.rs"], { cache: true }],
    [
      "runner changes publish the image before the backend lane",
      ["runner-runtime/src/main.rs"],
      { checksImage: true, worker: true },
    ],
    [
      "toolchain changes publish the checks image and rebuild Rust services",
      ["rust-toolchain.toml"],
      {
        checksImage: true,
        cache: true,
        worker: true,
        mediaWorker: true,
        router: true,
        media: true,
        api: true,
        cli: true,
        cliDistribution: true,
      },
    ],
    ["web-only changes deploy only web", ["web/src/routes/+page.svelte"], { web: true }],
    [
      "API implementation changes validate CLI without rebuilding distribution targets",
      ["api/src/main.rs"],
      { api: true, web: true, cli: true },
    ],
    ["router changes deploy the Git router", ["repo-router/src/main.rs"], { router: true }],
    [
      "CLI prebuilt launcher selects the CLI service",
      ["deploy/railway/start-prebuilt.sh"],
      { media: true, cli: true },
    ],
    [
      "backend runtime image selects every backend service",
      ["deploy/railway/prebuilt.Dockerfile"],
      { cache: true, worker: true, router: true, api: true },
    ],
    [
      "web runtime image selects the web service",
      ["deploy/railway/web.Dockerfile"],
      { web: true },
    ],
    [
      "CLI prebuilt config selects CLI without rebuilding distributions",
      ["deploy/railway/prebuilt-cli.railpack.json"],
      { cli: true },
    ],
    [
      "CLI tests validate CLI without rebuilding distribution targets",
      ["cli/tests/request.rs"],
      { cli: true },
    ],
    [
      "CLI source changes rebuild distribution targets",
      ["cli/src/request.rs"],
      { cli: true, cliDistribution: true },
    ],
    [
      "distribution config changes rebuild distribution targets",
      ["cli/distribution/targets.json"],
      { cli: true, cliDistribution: true },
    ],
    [
      "distribution selector changes run CLI validation and rebuild distribution targets",
      [".github/scripts/select-cli-distribution-targets.mjs"],
      { cli: true, cliDistribution: true },
    ],
    [
      "unrelated shared crates retain broad CLI validation without rebuilding targets",
      ["crates/scope-cache-contract/src/lib.rs"],
      {
        checksImage: true,
        cache: true,
        worker: true,
        mediaWorker: true,
        router: true,
        media: true,
        api: true,
        web: true,
        cli: true,
      },
    ],
    [
      "shared workspace changes preserve the previous conservative scope",
      ["crates/scope-domain/src/lib.rs"],
      allLanes,
    ],
    [
      "conductor changes exercise every lane",
      [".github/workflows/scope-production-deploy.yml"],
      allLanes,
    ],
    [
      "production health policy changes exercise every lane",
      [".github/scripts/railway-service-health.mjs"],
      allLanes,
    ],
  ];

  for (const [name, paths, lanes] of cases) {
    assert.deepEqual(classifyChanges(manifest, paths), deploymentSelection(lanes), name);
  }
});

test("manual component and all scopes are explicit", () => {
  assert.deepEqual(classifyChanges(manifest, [], "web"), deploymentSelection({ web: true }));
  assert.deepEqual(
    classifyChanges(manifest, [], "cli"),
    deploymentSelection({ cli: true, cliDistribution: true }),
  );
  assert.ok(Object.values(classifyChanges(manifest, [], "all")).every(Boolean));
  assert.throws(
    () => classifyChanges(manifest, [], "cliDistribution"),
    /Unknown deployment scope/,
  );
  assert.throws(() => classifyChanges(manifest, [], "database"), /Unknown deployment scope/);
});

test("planner emits the CLI distribution selection as a snake-case workflow output", () => {
  const output = execFileSync(process.execPath, [
    fileURLToPath(new URL("./plan-production-deployment.mjs", import.meta.url)),
    "--manifest",
    fileURLToPath(new URL("../deployment-services.json", import.meta.url)),
    "--scope",
    "cli",
  ], {
    encoding: "utf8",
    env: {
      ...process.env,
      GITHUB_OUTPUT: "",
      GITHUB_STEP_SUMMARY: "",
    },
  });

  assert.match(output, /^cli=true$/m);
  assert.match(output, /^cli_distribution=true$/m);
  assert.doesNotMatch(output, /^cliDistribution=/m);
});

test("an unseeded production ledger deploys every component", () => {
  assert.ok(Object.values(planFromDeploymentProgress(manifest, {})).every(Boolean));
});

test("skipped components remain selected across a later backend-only change", () => {
  const selection = planFromDeploymentProgress(manifest, {
    checksImage: [],
    cache: ["cache-service/src/main.rs"],
    worker: [],
    mediaWorker: [],
    router: [],
    media: [],
    api: [],
    // Web last succeeded before commit A. Its component-specific range still includes A's
    // web change when commit B changes only the cache service after A's web job was skipped.
    web: ["web/src/routes/+page.svelte", "cache-service/src/main.rs"],
    cli: [],
  });

  assert.deepEqual(selection, {
    checksImage: false,
    cache: true,
    worker: false,
    mediaWorker: false,
    router: false,
    media: false,
    api: false,
    web: true,
    cli: false,
    cliDistribution: false,
  });
});

test("skipped validation ancestors do not suppress a selected backend deployment", () => {
  assert.match(
    productionJobCondition("backend-deploy"),
    /^!cancelled\(\) && github\.event_name/,
  );
});

test("web and CLI deployment conditions are cancellation-safe after optional backend jobs", () => {
  const conditions = Object.fromEntries(["web-deploy", "cli-deploy"].map((job) => [
    job,
    productionJobCondition(job),
  ]));
  for (const condition of Object.values(conditions)) {
    assert.match(condition, /!cancelled\(\) && github\.event_name/);
    assert.match(condition, /needs\.production-validation-gate\.result == 'success'/);
    assert.match(condition, /needs\.backend-deploy\.result == 'success'/);
    assert.match(condition, /needs\.backend-deploy\.result == 'skipped'/);
  }

});

test("frontend production deployment eligibility covers optional backend and failure states", () => {
  const fixtures = [
    ["backend selected", { backendSelected: true, backendResult: "success" }, true],
    ["backend skipped", { backendSelected: false, backendResult: "skipped" }, true],
    [
      "failed validation",
      { backendSelected: false, backendResult: "skipped", validationResult: "failure" },
      false,
    ],
    ["failed backend", { backendSelected: true, backendResult: "failure" }, false],
    [
      "canceled workflow",
      { backendSelected: false, backendResult: "skipped", cancelled: true },
      false,
    ],
  ];

  for (const job of ["web-deploy", "cli-deploy"]) {
    const condition = productionJobCondition(job);
    for (const [name, input, expected] of fixtures) {
      assert.equal(
        evaluateProductionCondition(condition, productionConditionContext(input)),
        expected,
        `${job}: ${name}`,
      );
    }
  }
});

test("CLI publication waits for a selected application's release proof", () => {
  const condition = productionJobCondition("cli-deploy");
  for (const proofResult of ["failure", "cancelled", "skipped"]) {
    assert.equal(evaluateProductionCondition(condition, productionConditionContext({
      backendSelected: false, webSelected: true, stagingResult: proofResult,
    })), false, `web and CLI cannot publish after ${proofResult} proof`);
    assert.equal(evaluateProductionCondition(condition, productionConditionContext({
      backendSelected: true, backendResult: "success", stagingResult: proofResult,
    })), false, `backend and CLI cannot publish after ${proofResult} proof`);
  }
  assert.equal(evaluateProductionCondition(condition, productionConditionContext({
    backendSelected: false, webSelected: false, stagingResult: "skipped",
  })), true, "CLI-only publication has no application rehearsal");
  assert.equal(evaluateProductionCondition(condition, productionConditionContext({
    backendSelected: false, webSelected: true, stagingResult: "success",
  })), true, "web and CLI publish after successful proof");
  assert.match(productionWorkflow, /cli-deploy:\n    name: CLI deploy\n    needs: \[plan, production-validation-gate, release-staging-proof, backend-deploy\]/);
});

test("the final production gate verifies selected and carried-forward services", () => {
  const condition = productionJobCondition("production-health-gate");
  assert.match(condition, /^!cancelled\(\) && github\.event_name/);
  const fixtures = [
    [
      "all Railway components carried forward",
      { backendSelected: false, cliSelected: false, webSelected: false },
      true,
    ],
    [
      "backend selected and frontend carried forward",
      {
        backendSelected: true,
        backendResult: "success",
        cliSelected: false,
        webSelected: false,
      },
      true,
    ],
    [
      "selected web deployment failed",
      { backendSelected: false, cliSelected: false, webResult: "failure" },
      false,
    ],
    [
      "selected backend deployment failed",
      {
        backendSelected: true,
        backendResult: "failure",
        cliSelected: false,
        webSelected: false,
      },
      false,
    ],
    [
      "workflow canceled after deploy jobs",
      { backendSelected: false, cancelled: true, cliSelected: false, webSelected: false },
      false,
    ],
  ];
  for (const [name, input, expected] of fixtures) {
    assert.equal(
      evaluateProductionCondition(condition, productionConditionContext(input)),
      expected,
      name,
    );
  }
});

test("CLI deployment progress selects distribution builds only for binary inputs", () => {
  const broadOnly = planFromDeploymentProgress(manifest, {
    checksImage: [],
    cache: [],
    worker: [],
    mediaWorker: [],
    router: [],
    media: [],
    api: [],
    web: [],
    cli: ["api/src/main.rs"],
  });
  const binaryChange = planFromDeploymentProgress(manifest, {
    checksImage: [],
    cache: [],
    worker: [],
    mediaWorker: [],
    router: [],
    media: [],
    api: [],
    web: [],
    cli: ["crates/scope-api-contract/src/lib.rs"],
  });

  assert.deepEqual(broadOnly, deploymentSelection({ cli: true }));
  assert.deepEqual(
    binaryChange,
    deploymentSelection({ cli: true, cliDistribution: true }),
  );
});

test("manual scopes ignore pending production components", () => {
  assert.deepEqual(planFromDeploymentProgress(manifest, {}, "web"), {
    checksImage: false,
    cache: false,
    worker: false,
    mediaWorker: false,
    router: false,
    media: false,
    api: false,
    web: true,
    cli: false,
    cliDistribution: false,
  });
});

test("deployment manifest is a single coherent production graph", () => {
  const order = ["cache", "worker", "router", "media", "mediaWorker", "api", "web", "cli"];
  const serviceIds = order.map((service) => manifest.services[service].id).filter(Boolean);

  assert.equal(manifest.deploymentAuthority, "github-actions");
  assert.equal(manifest.source.nativeAutodeploy, false);
  assert.equal(new Set(serviceIds).size, serviceIds.length);
  assert.match(manifest.services.media.id, /^[0-9a-f-]{36}$/);
  assert.match(manifest.services.mediaWorker.id, /^[0-9a-f-]{36}$/);
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
    "worker/railway.json": "/healthz",
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

test("prepared web and backend jobs cannot build after activation begins", () => {
  const preparation = readFileSync(new URL("../workflows/scope-release-prepare.yml", import.meta.url), "utf8");
  assert.match(
    productionWorkflow,
    /maintenance_budget_seconds: \$\{\{ fromJSON\(format\('\{0\}', inputs\.maintenance_budget_seconds \|\| 0\)\) \}\}/,
  );
  for (const workflow of [backendDeployWorkflow, webDeployWorkflow]) {
    assert.match(workflow, /name: prepared-release-\$\{\{ inputs\.source_sha \}\}/);
    assert.match(workflow, /SCOPE_PREPARED_RELEASE_PATH: prepared-release\.json/);
    assert.doesNotMatch(workflow, /cargo build|docker build|railway up|pnpm build/);
  }
  assert.match(preparation, /prepare-railway-artifact\.sh/);
  assert.match(backendCiWorkflow, /name: backend-release-\$\{\{ github\.sha \}\}/);
  assert.match(backendDeployWorkflow, /extract-railway-maintenance\.sh prepared-release\.json/);
  assert.doesNotMatch(backendDeployWorkflow, /backend-release-\$\{\{ inputs\.source_sha \}\}/);
  assert.match(preparation, /cutover-restore/);
  assert.match(preparation.split("\njobs:")[0], /actions: read/);
  assert.match(cliDeployWorkflow, /cp cli\/railway\.json \.railway-upload\/railway\.json/);
  assert.doesNotMatch(cliDeployWorkflow, /cargo build/);
});

test("web-only releases prepare artifacts when the backend image job is skipped", () => {
  const condition = productionJobCondition("release-preparation");
  const fixtures = [
    ["web-only", { backendSelected: false, cliSelected: false }, true],
    ["backend and web", { backendSelected: true }, true],
    ["backend-only", { backendSelected: true, webSelected: false }, true],
    ["no application selected", { backendSelected: false, webSelected: false }, false],
    ["failed image build", { backendSelected: true, mediaWorkerImageResult: "failure", validationResult: "skipped" }, false],
    ["failed validation", { backendSelected: false, validationResult: "failure" }, false],
    ["cancelled", { backendSelected: false, cancelled: true }, false],
    ["pull request", { backendSelected: false, eventName: "pull_request" }, false],
  ];
  for (const [name, overrides, expected] of fixtures) {
    assert.equal(evaluateProductionCondition(condition, productionConditionContext(overrides)), expected, name);
  }
});

test("application activation requires completed preparation and staging proof", () => {
  for (const job of ["backend-deploy", "web-deploy"]) {
    const condition = productionJobCondition(job);
    const ready = { backendSelected: true, backendResult: "success" };
    assert.equal(evaluateProductionCondition(condition, productionConditionContext(ready)), true);
    for (const failure of ["failure", "cancelled", "skipped"]) {
      assert.equal(evaluateProductionCondition(condition, productionConditionContext({ ...ready, preparationResult: failure })), false);
      assert.equal(evaluateProductionCondition(condition, productionConditionContext({ ...ready, stagingResult: failure })), false);
    }
  }
  assert.equal(evaluateProductionCondition(productionJobCondition("backend-deploy"), productionConditionContext({
    backendSelected: true, recoveryId: "123", stagingResult: "skipped",
  })), true, "pinned recovery must not delay reopening for another staging run");
});

test("automatic code events and failed release proof cannot activate production", () => {
  for (const job of ["backend-deploy", "web-deploy"]) {
    for (const eventName of ["push", "pull_request", "schedule", "workflow_dispatch"]) {
      const context = { eventName, backendSelected: true, backendResult: "success" };
      assert.equal(evaluateProductionCondition(productionJobCondition(job), productionConditionContext(context)),
        ["schedule", "workflow_dispatch"].includes(eventName));
      assert.equal(evaluateProductionCondition(productionJobCondition(job), productionConditionContext({
        ...context, stagingResult: "skipped",
      })), false);
    }
  }
});

test("daily releases use Chicago time and leave push events out of the workflow", () => {
  const triggers = productionWorkflow.split("\nconcurrency:")[0];
  assert.match(triggers, /schedule:\n    - cron: "0 9 \* \* \*"\n      timezone: America\/Chicago/);
  assert.match(triggers, /  workflow_dispatch:/);
  assert.match(triggers, /  pull_request:/);
  assert.doesNotMatch(triggers, /  push:|skip_staging_rehearsal/);
  assert.match(triggers, /default: changed/);
  assert.doesNotMatch(productionWorkflow, /\n  staging-proof:/);
  assert.match(productionWorkflow, /-f target_environment=release-proof/);
  assert.equal(manifest.source.nativeAutodeploy, false);
  assert.equal(manifest.railway.staging.environmentName, "release-proof");
});

test("Node workflows cache pnpm and browser downloads by the web lockfile", () => {
  for (const workflow of [integrationCiWorkflow, rustChecksWorkflow, webCiWorkflow]) {
    assert.match(
      workflow,
      /uses: pnpm\/action-setup@[0-9a-f]{40} # v5/,
    );
    assert.match(workflow, /cache: pnpm/);
    assert.match(workflow, /cache-dependency-path: web\/pnpm-lock\.yaml/);
  }

  assert.match(integrationCiWorkflow, /path: ~\/\.cache\/ms-playwright/);
  assert.match(
    integrationCiWorkflow,
    /key: playwright-\$\{\{ runner\.os \}\}-\$\{\{ hashFiles\('web\/pnpm-lock\.yaml'\) \}\}/,
  );
});


test("production success follows the complete monitored transition", () => {
  for (const workflow of [backendDeployWorkflow, webDeployWorkflow]) {
    const recordStep = workflow.slice(workflow.indexOf("      - name: Record successful Railway"));
    assert.match(recordStep, /if: steps\.transition\.outcome == 'success'/);
    assert.match(workflow, /name: Deploy to Railway\n\s+id: transition/);
  }
});

test("staging dispatch has a unique identity beyond the candidate SHA", () => {
  const proof = productionWorkflow.slice(productionWorkflow.indexOf("  release-staging-proof:"), productionWorkflow.indexOf("  backend-deploy:"));
  assert.match(proof, /proof_id="\$\(cat \/proc\/sys\/kernel\/random\/uuid\)"/);
  assert.match(proof, /title="Scope release-proof \$SOURCE_SHA \/ \$proof_id"/);
  assert.match(proof, /-f proof_request_id="\$proof_id"/);
  assert.match(stagingWorkflow, /run-name:.*inputs\.proof_request_id/);
});


test("release-proof watcher covers preparation, proof, and cleanup budgets", () => {
  const proof = productionWorkflow.slice(productionWorkflow.indexOf("  release-staging-proof:"), productionWorkflow.indexOf("  backend-deploy:"));
  const parentBudget = Number(proof.match(/timeout-minutes: (\d+)/)[1]);
  const childBudget = ["prepare", "prove", "cleanup"].reduce((total, name) => {
    const job = stagingWorkflow.split(`\n  ${name}:\n`)[1].split(/\n  [\w-]+:\n/)[0];
    return total + Number(job.match(/timeout-minutes: (\d+)/)[1]);
  }, 0);
  assert(parentBudget > childBudget, "watcher must allow the child workflow and dispatch overhead to finish");
});

test("recovery validates provenance before selecting its source revision", () => {
  const selection = productionWorkflow.slice(productionWorkflow.indexOf("      - name: Select immutable release revision"), productionWorkflow.indexOf("      - name: Read successful production revisions"));
  assert(selection.indexOf("cutover-validate-recovery") < selection.indexOf('echo "sha=$RECOVER_SHA"'));
  assert.match(selection, /test "\$GITHUB_REF" = refs\/heads\/main/);
  assert.match(backendDeployWorkflow, /ref: \$\{\{ github\.sha \}\}\n\s+persist-credentials: false/);
  assert.match(productionWorkflow.split("\njobs:")[0], /deployments: read/);
  assert.doesNotMatch(productionWorkflow.split("\njobs:")[0], /: write/);
  const checks = readFileSync(new URL("../workflows/scope-checks-image.yml", import.meta.url), "utf8");
  const candidate = checks.slice(checks.indexOf("  validate:"), checks.indexOf("  build:"));
  assert.match(candidate, /if: github\.event_name == 'pull_request'/);
  assert.doesNotMatch(candidate, /: write/);
  assert.match(checks.slice(checks.indexOf("  build:")), /if: github\.event_name != 'pull_request'/);
});
