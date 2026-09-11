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
const workflow = name => readFileSync(new URL(`../workflows/${name}.yml`, import.meta.url), 'utf8');
const productionWorkflow = workflow('release');
const backendCiWorkflow = workflow('scope-api-ci');
const backendDeployWorkflow = workflow('deploy-backend');
const cliDeployWorkflow = workflow('publish-cli');
const webDeployWorkflow = workflow('deploy-web');

const lanes = ['checks-image', 'cache', 'run-worker', 'media-worker', 'git-router', 'media-api',
  'api', 'web', 'cli-downloads', 'cli-distribution'];
function deploymentSelection(overrides = {}) {
  return { ...Object.fromEntries(lanes.map(lane => [lane, false])), ...overrides };
}
function deploymentChanges(overrides = {}) {
  return { ...Object.fromEntries(lanes.filter(lane => lane !== 'cli-distribution').map(lane => [lane, []])), ...overrides };
}

test("changes select the required deployment lanes", () => {
  const cases = [
    ['license changes rebuild all distributions', 'LICENSE', lanes],
    ['attribution changes rebuild all distributions', 'NOTICE', lanes],
    ['dependency notices rebuild all distributions', 'legal/third-party-rust.txt', lanes],
    ['documentation-only changes do not deploy', 'docs/cache.md', []],
    ['cache service changes run backend only', 'cache-service/src/main.rs', ['cache']],
    ['runner changes publish the image before the backend lane', 'runner-runtime/src/main.rs', ['checks-image', 'run-worker']],
    ['toolchain changes rebuild Rust services', 'rust-toolchain.toml', lanes.filter(lane => lane !== 'web')],
    ['web-only changes deploy only web', 'web/src/routes/+page.svelte', ['web']],
    ['API changes validate CLI without rebuilding distributions', 'api/src/main.rs', ['api', 'web', 'cli-downloads']],
    ['router changes deploy the Git router', 'repo-router/src/main.rs', ['git-router']],
    ['CLI prebuilt launcher selects the CLI service', 'deploy/railway/start-prebuilt.sh', ['media-api', 'cli-downloads']],
    ['backend runtime selects backend services', 'deploy/railway/prebuilt.Dockerfile', ['cache', 'run-worker', 'git-router', 'api']],
    ['web runtime selects web', 'deploy/railway/web.Dockerfile', ['web']],
    ['CLI prebuilt config skips distribution builds', 'deploy/railway/prebuilt-cli.railpack.json', ['cli-downloads']],
    ['CLI tests skip distribution builds', 'cli/tests/request.rs', ['cli-downloads']],
    ['CLI source rebuilds distributions', 'cli/src/request.rs', ['cli-downloads', 'cli-distribution']],
    ['distribution config rebuilds distributions', 'cli/distribution/targets.json', ['cli-downloads', 'cli-distribution']],
    ['distribution selector validates and rebuilds CLI', '.github/scripts/select-cli-distribution-targets.mjs', ['cli-downloads', 'cli-distribution']],
    ['unrelated shared crates validate CLI without rebuilding distributions', 'crates/scope-cache-contract/src/lib.rs', lanes.filter(lane => lane !== 'cli-distribution')],
    ['shared workspace changes retain conservative scope', 'crates/scope-domain/src/lib.rs', lanes],
    ['conductor changes exercise every lane', '.github/workflows/release.yml', lanes],
    ['production health policy exercises every lane', '.github/scripts/railway-service-health.mjs', lanes],
  ];
  for (const [name, path, selected] of cases) {
    const expected = deploymentSelection(Object.fromEntries(selected.map(lane => [lane, true])));
    assert.deepEqual(classifyChanges(manifest, [path]), expected, name);
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
  for (const [scope, required] of [
    ['cli-downloads', ['cli=true', 'cli_distribution=true', 'backend_selected=false']],
    ['api', ['api=true', 'backend_selected=true']],
  ]) {
    const output = execFileSync(process.execPath, [
      fileURLToPath(new URL('./plan-production-deployment.mjs', import.meta.url)), '--scope', scope,
    ], { encoding: 'utf8', env: { ...process.env, GITHUB_OUTPUT: '', GITHUB_STEP_SUMMARY: '' } });
    for (const value of required) assert.ok(output.split('\n').includes(value), value);
  }
});

test("an unseeded production ledger deploys every component", () => {
  assert.ok(Object.values(planFromDeploymentProgress(manifest, {})).every(Boolean));
});

test("skipped components remain selected across a later backend-only change", () => {
  const selection = planFromDeploymentProgress(manifest, deploymentChanges({
    cache: ["cache-service/src/main.rs"],
    // Web last succeeded before commit A. Its component-specific range still includes A's
    // web change when commit B changes only the cache service after A's web job was skipped.
    web: ["web/src/routes/+page.svelte", "cache-service/src/main.rs"],
    "cli-downloads": [],
  }));

  assert.deepEqual(selection, deploymentSelection({ cache: true, web: true }));
});

test("CLI deployment progress selects distribution builds only for binary inputs", () => {
  const broadOnly = planFromDeploymentProgress(manifest, deploymentChanges({
    "cli-downloads": ["api/src/main.rs"],
  }));
  const binaryChange = planFromDeploymentProgress(manifest, deploymentChanges({
    "cli-downloads": ["crates/scope-api-contract/src/lib.rs"],
  }));

  assert.deepEqual(broadOnly, deploymentSelection({ "cli-downloads": true }));
  assert.deepEqual(
    binaryChange,
    deploymentSelection({ "cli-downloads": true, "cli-distribution": true }),
  );
});

test("manual scopes ignore pending production components", () => {
  assert.deepEqual(planFromDeploymentProgress(manifest, {}, "web"), deploymentSelection({ web: true }));
});

test("prepared web and backend jobs cannot build after activation begins", () => {
  const preparation = readFileSync(new URL("../workflows/prepare-release.yml", import.meta.url), "utf8");
  assert.doesNotMatch(productionWorkflow, /maintenance_budget_seconds/);
  for (const workflow of [backendDeployWorkflow, webDeployWorkflow]) {
    assert.match(workflow, /name: prepared-release-\$\{\{ inputs\.source_sha \}\}/);
    assert.match(workflow, /SCOPE_PREPARED_RELEASE_PATH: prepared-release\.json/);
    assert.doesNotMatch(workflow, /cargo build|docker build|railway up|pnpm build/);
  }
  assert.match(preparation, /prepare-railway-artifact\.sh/);
  assert.match(backendCiWorkflow, /name: backend-release-\$\{\{ github\.sha \}\}/);
  assert.match(backendDeployWorkflow, /extract-railway-maintenance\.sh prepared-release\.json/);
  assert.doesNotMatch(backendDeployWorkflow, /backend-release-\$\{\{ inputs\.source_sha \}\}/);
  assert.match(preparation, /selected-release-\$\{\{ inputs\.source_sha \}\}/);
  assert.match(preparation.split("\njobs:")[0], /actions: read/);
  assert.match(cliDeployWorkflow, /cp cli\/railway\.json \.railway-upload\/railway\.json/);
  assert.doesNotMatch(cliDeployWorkflow, /cargo build/);
});

test("production success follows the complete monitored transition", () => {
  for (const workflow of [backendDeployWorkflow, webDeployWorkflow]) {
    const recordStep = workflow.slice(workflow.indexOf("      - name: Record successful Railway"));
    assert.match(recordStep, /if: steps\.transition\.outcome == 'success'/);
    assert.match(workflow, /name: Deploy to Railway\n\s+id: transition/);
  }
});

test("release selection uses the trusted control revision before exposing a source revision", () => {
  const requireMain = productionWorkflow.indexOf('- name: Require main for releases');
  const selection = productionWorkflow.indexOf('run: node .github/scripts/release-selection.mjs');
  const retain = productionWorkflow.indexOf('- name: Retain selected immutable release');
  assert(requireMain >= 0 && selection > requireMain && retain > selection);
  assert.match(productionWorkflow.slice(requireMain, selection), /test "\$GITHUB_REF" = refs\/heads\/main/);
  assert.match(backendDeployWorkflow, /ref: \$\{\{ github\.sha \}\}\n\s+persist-credentials: false/);
  assert.match(productionWorkflow.split("\njobs:")[0], /deployments: read/);
  assert.doesNotMatch(productionWorkflow.split("\njobs:")[0], /: write/);
  const checks = readFileSync(new URL("../workflows/scope-checks-image.yml", import.meta.url), "utf8");
  const candidate = checks.slice(checks.indexOf("  validate:"), checks.indexOf("  build:"));
  assert.match(candidate, /if: github\.event_name == 'pull_request'/);
  assert.doesNotMatch(candidate, /: write/);
  assert.match(checks.slice(checks.indexOf("  build:")), /if: github\.event_name != 'pull_request'/);
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
