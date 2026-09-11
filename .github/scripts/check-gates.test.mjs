import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, cpSync, mkdtempSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import test from 'node:test';
import { classifyChanges } from './plan-production-deployment.mjs';

const root = resolve(import.meta.dirname, '../..');
const read = (path) => readFileSync(resolve(root, path), 'utf8');
const manifest = JSON.parse(read('.github/deployment-services.json'));
const gates = ['backend', 'cli', 'web', 'contract', 'policy', 'integration', 'ops'];

// Capture the commands actually executed, without requiring installed toolchains,
// credentials, or a running stack. The scripts remain the command inventory.
function commands(gate, ...args) {
  const dir = mkdtempSync(resolve(tmpdir(), 'scope-gates-'));
  try {
    for (const tool of ['cargo', 'pnpm', 'node', 'bash', 'python3']) {
      writeFileSync(resolve(dir, tool), '#!/bin/sh\ncase "$1" in *dev/checks/*) exec /bin/bash "$@" ;; esac\nprintf "%s" "$(basename "$0")"\nprintf " %s" "$@"\nprintf "\\n"\n', { mode: 0o755 });
    }
    return execFileSync('/bin/bash', [resolve(root, `dev/checks/${gate}`), ...args], {
      env: { ...process.env, PATH: `${dir}:${process.env.PATH}` }, encoding: 'utf8',
    }).trim().split('\n');
  } finally { rmSync(dir, { recursive: true, force: true }); }
}

test('backend variants preserve API feature coverage explicitly', () => {
  const withApi = commands('backend', 'with-api');
  const withoutApi = commands('backend', 'without-api');
  assert.ok(withApi.includes('cargo test --workspace --features api/test-support --locked'));
  assert.ok(withApi.includes('cargo test -p api --features local-dev --locked dev::'));
  assert.ok(withoutApi.includes('cargo test --workspace --exclude api --locked'));
  assert.ok(withoutApi.every((line) => !line.includes('--features')));
  assert.equal(spawnSync('dev/checks/backend', ['invalid'], { cwd: root }).status, 2);
});

test('web gate includes contract, observer, and resource rules; CLI and integration retain their coverage', () => {
  assert.deepEqual(commands('web'), [
    'pnpm test', 'pnpm check', 'pnpm build',
  ]);
  const webChecks = JSON.parse(read('web/package.json')).scripts.check;
  assert.equal(webChecks, 'pnpm typecheck && ../dev/checks/contract && pnpm check:observer-boundary && pnpm check:resource-boundary && pnpm check:react-doctor && pnpm check:konsistent');
  assert.deepEqual(commands('contract'), ['pnpm check:api-contract']);
  assert.ok(commands('cli').includes('cargo build --manifest-path cli/Cargo.toml --release --locked --bin scope --bin scope-cli-service'));
  assert.deepEqual(commands('integration', 'cli'), ['cargo test --manifest-path cli/Cargo.toml --test contribution_flow --locked -- --ignored --nocapture']);
  assert.deepEqual(commands('integration', 'web'), ['pnpm test:smoke']);
});

test('local and both CI callers use the shared inventory', () => {
  const github = ['rust-workspace-checks', 'scope-api-ci', 'scope-cli-build', 'scope-web-ci', 'ci', 'release', 'scope-integration-ci']
    .map((name) => read(`.github/workflows/${name}.yml`)).join('\n');
  const scope = read('.scope/runs/checks.yml');
  for (const gate of gates.filter((gate) => gate !== 'contract')) {
    assert.ok(github.includes(`dev/checks/${gate}`), `GitHub: ${gate}`);
    assert.ok(scope.includes(`dev/checks/${gate}`), `Scope: ${gate}`);
  }
  assert.ok(read('dev/check').includes('dev/checks/policy'));
  assert.ok(read('web/package.json').includes('dev/checks/contract'));
  assert.ok(github.includes('dev/checks/contract'));
});

test('every deployment and policy script test is run by a shared gate', () => {
  const invoked = ['ops', 'policy', 'cli'].flatMap((gate) => commands(gate));
  for (const name of readdirSync(resolve(root, '.github/scripts'))) {
    if (!name.endsWith('.test.mjs')) continue;
    const path = `.github/scripts/${name}`;
    assert.ok(invoked.some((command) => command.startsWith('node --test ') && command.split(' ').includes(path)), `${path} has no test gate`);
  }
});

test('gate inputs select checks through change scopes', () => {
  const paths = execFileSync('git', ['ls-files', '--cached', '--others', '--exclude-standard'], { cwd: root, encoding: 'utf8' }).trim().split('\n');
  const gateInputs = paths.filter((path) => existsSync(resolve(root, path)) && /^(dev\/|\.github\/scripts\/|bench\/|deploy\/aws\/)/.test(path));
  gateInputs.push('.scope/runs/checks.yml', '.github/source-size-audit.json');
  for (const path of gateInputs) {
    const selected = classifyChanges(manifest, [path]);
    assert.ok(Object.values(selected).some(Boolean), `${path} must select checks`);
  }
});

test('policy rejects oversized non-web source in a complete checkout', () => {
  assert.ok(commands('policy').includes('python3 dev/licensing/generate.py --check'));
  assert.ok(commands('policy').includes('node .github/scripts/check-source-size.mjs'));
  const dir = mkdtempSync(resolve(tmpdir(), 'scope-size-gate-'));
  try {
    mkdirSync(resolve(dir, '.github/scripts'), { recursive: true });
    mkdirSync(resolve(dir, 'worker/src'), { recursive: true });
    cpSync(resolve(root, '.github/scripts/check-source-size.mjs'), resolve(dir, '.github/scripts/check-source-size.mjs'));
    writeFileSync(resolve(dir, '.github/source-size-audit.json'), JSON.stringify({ version: 1, production: [] }));
    writeFileSync(resolve(dir, 'worker/src/oversized.rs'), '// outside web\n'.repeat(1001));
    execFileSync('git', ['init', '--quiet', dir]);
    const result = spawnSync(process.execPath, ['.github/scripts/check-source-size.mjs'], { cwd: dir, encoding: 'utf8' });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /worker\/src\/oversized.rs/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test('CLI release identity survives subsequent Cargo commands and cross containers', () => {
  const workflow = read('.github/workflows/scope-cli-build.yml');
  const build = workflow.slice(workflow.indexOf('\n  build:\n'));
  const jobSettings = build.slice(0, build.indexOf('\n    steps:'));
  assert.match(jobSettings, /\n    env:\n      SCOPE_BUILD_SHA: \$\{\{ github.sha \}\}/);
  assert.equal((build.match(/^\s+SCOPE_BUILD_SHA:/gm) ?? []).length, 1, 'steps must inherit the job SHA');
  assert.match(jobSettings, /CROSS_CONFIG: \$\{\{ github.workspace \}\}\/Cross.toml/);
  assert.match(read('Cross.toml'), /\[build.env\]\s+passthrough = \["SCOPE_BUILD_SHA"\]/);
  const selected = classifyChanges(manifest, ['Cross.toml']);
  assert.equal(selected["cli-downloads"], true);
  assert.equal(selected["cli-distribution"], true);
});

test('artifact staging rejects missing and stale release identities after all builds', () => {
  const workflow = read('.github/workflows/scope-cli-build.yml');
  const stage = workflow.split('      - name: Stage artifact\n')[1].split('      - name: Upload artifact\n')[0];
  const script = stage.split('        run: |\n')[1].replace(/^          /gm, '');
  const sha = '1234567890abcdef1234567890abcdef12345678';
  const dir = mkdtempSync(resolve(tmpdir(), 'scope-release-identity-'));
  try {
    const run = (smoke, output, embedded = output, buildSha = sha) => {
      rmSync(resolve(dir, 'dist'), { recursive: true, force: true });
      writeFileSync(resolve(dir, 'scope'), `#!/bin/sh\n# ${embedded}\nprintf '%s\\n' '${output}'\n`, { mode: 0o755 });
      return spawnSync('bash', ['-e', '-o', 'pipefail', '-c', script
        .replaceAll('${{ matrix.binary }}', 'scope')
        .replaceAll('${{ matrix.artifact }}', 'scope-release')
        .replaceAll('${{ matrix.smoke }}', String(smoke))], {
        cwd: dir, env: { ...process.env, SCOPE_BUILD_SHA: buildSha }, encoding: 'utf8',
      });
    };
    const current = `scope 0.1.0 (build ${sha}; protocol 1)`;
    for (const smoke of [true, false]) {
      assert.equal(run(smoke, current).status, 0);
      assert.match(readFileSync(resolve(dir, 'dist/scope-release'), 'utf8'), new RegExp(sha));
      assert.notEqual(run(smoke, 'scope 0.1.0 (build development; protocol 1)').status, 0);
      assert.throws(() => readFileSync(resolve(dir, 'dist/scope-release')), /ENOENT/);
      assert.notEqual(run(smoke, current, current, '').status, 0);
    }
    assert.notEqual(run(true, 'scope 0.1.0 (build development; protocol 1)', sha).status, 0);
    assert.notEqual(run(true, `scope 0.1.0 (build ${sha})`).status, 0);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

function releaseJobs() {
  const workflow = read('.github/workflows/release.yml');
  return Object.fromEntries([...workflow.matchAll(/^  ([\w-]+):\n([\s\S]*?)(?=^  [\w-]+:|$(?![\s\S]))/gm)]
    .map(([, name, body]) => [name, { if: body.match(/^    if: >-\n((?:      .*\n)+)/m)?.[1].trim() }]));
}

function releasePath(selected, { reuse = false, resumeStaging = false, failure = '', cancelled = false, ref = 'refs/heads/main', backendActivatesWeb = false } = {}) {
  const jobs = releaseJobs();
  const outputs = Object.fromEntries(['checks_image', 'cache', 'worker', 'media_worker', 'router', 'media', 'api', 'web', 'cli']
    .map((key) => [key, String(selected.includes(key))]));
  outputs.prepared_run_id = reuse || resumeStaging ? '123' : '';
  outputs.recover_cutover_id = reuse ? '456' : '';
  outputs.resume_staging = String(resumeStaging);
  const needs = { plan: { result: 'success', outputs }, validation: { result: 'success' } };
  for (const key of ['release-preparation', 'production-preflight', 'staging', 'backend-deploy', 'web-deploy', 'cli-deploy', 'production-health-gate']) {
    const expression = jobs[key].if.replace(/needs\.([\w-]+)/g, 'needs["$1"]');
    const enabled = Function('needs', 'github', 'cancelled', `return (${expression});`)(needs, { ref }, () => cancelled);
    needs[key] = { result: enabled ? key === failure ? 'failure' : 'success' : 'skipped',
      outputs: key === 'backend-deploy' ? { web_activated: String(enabled && key !== failure && backendActivatesWeb) } : {} };
  }
  return needs;
}

test('release paths stage applications once and leave no-op and distribution-only polls cheap', () => {
  const empty = releasePath([]);
  for (const key of ['release-preparation', 'staging', 'backend-deploy', 'web-deploy', 'cli-deploy', 'production-health-gate']) assert.equal(empty[key].result, 'skipped', key);
  const web = releasePath(['web']);
  assert.equal(web.staging.result, 'success');
  assert.equal(web['production-preflight'].result, 'skipped');
  assert.equal(web['web-deploy'].result, 'success');
  assert.equal(web['production-health-gate'].result, 'success');
  const cli = releasePath(['cli']);
  assert.equal(cli.staging.result, 'skipped');
  assert.equal(cli['cli-deploy'].result, 'success');
  assert.equal(cli['production-health-gate'].result, 'success');
  const checks = releasePath(['checks_image']);
  assert.equal(checks.staging.result, 'skipped');
  assert.equal(checks['production-health-gate'].result, 'success');
});

test('failed preflight, staging or activation cannot publish a successful release', () => {
  for (const failure of ['production-preflight', 'staging', 'backend-deploy', 'web-deploy', 'cli-deploy']) {
    const result = releasePath(['api', 'web', 'cli'], { failure });
    assert.equal(result['production-health-gate'].result, 'skipped', failure);
  }
});

test('interrupted releases reuse images and finish through the same final receipt', () => {
  const result = releasePath(['api', 'worker', 'cache', 'router', 'media', 'media_worker', 'web'], { reuse: true });
  assert.equal(result['release-preparation'].result, 'success');
  assert.equal(result.staging.result, 'skipped');
  assert.equal(result['backend-deploy'].result, 'success');
  assert.equal(result['web-deploy'].result, 'success');
  assert.equal(result['production-health-gate'].result, 'success');
});

test('resumed staging reuses prepared images but must pass smoke before any production activation', () => {
  const selected = ['api', 'worker', 'cache', 'router', 'media', 'media_worker', 'web'];
  const result = releasePath(selected, { resumeStaging: true });
  assert.equal(result['production-preflight'].result, 'skipped');
  for (const job of ['release-preparation', 'staging', 'backend-deploy', 'web-deploy', 'production-health-gate']) {
    assert.equal(result[job].result, 'success', job);
  }
  for (const failure of ['release-preparation', 'staging']) {
    const blocked = releasePath(selected, { resumeStaging: true, failure });
    for (const job of ['backend-deploy', 'web-deploy', 'production-health-gate']) {
      assert.equal(blocked[job].result, 'skipped', `${failure}: ${job}`);
    }
  }
});


test('cancelled releases and non-main refs cannot activate or record production', () => {
  for (const options of [{ cancelled: true }, { ref: 'refs/heads/feature' }]) {
    const result = releasePath(['api', 'web', 'cli'], options);
    for (const key of ['backend-deploy', 'web-deploy', 'cli-deploy', 'production-health-gate']) assert.equal(result[key].result, 'skipped', key);
  }
});


test('maintenance publishes the backend-owned web receipt without deploying web twice', () => {
  const result = releasePath(['api', 'web', 'cli'], { backendActivatesWeb: true });
  assert.equal(result['backend-deploy'].result, 'success');
  assert.equal(result['web-deploy'].result, 'skipped');
  assert.equal(result['production-health-gate'].result, 'success');
  const failed = releasePath(['api', 'web'], { backendActivatesWeb: true, failure: 'backend-deploy' });
  assert.equal(failed['production-health-gate'].result, 'skipped');
});

test('prepared web and backend jobs cannot build after activation begins', () => {
  const preparation = read('.github/workflows/prepare-release.yml');
  const backendDeploy = read('.github/workflows/deploy-backend.yml');
  for (const workflow of [backendDeploy, read('.github/workflows/deploy-web.yml')]) {
    assert.match(workflow, /name: prepared-release-\$\{\{ inputs\.source_sha \}\}/);
    assert.match(workflow, /SCOPE_PREPARED_RELEASE_PATH: prepared-release\.json/);
    assert.doesNotMatch(workflow, /cargo build|docker build|railway up|pnpm build/);
  }
  assert.match(preparation, /prepare-railway-artifact\.sh/);
  assert.match(read('.github/workflows/scope-api-ci.yml'), /name: backend-release-\$\{\{ github\.sha \}\}/);
  assert.match(backendDeploy, /extract-railway-maintenance\.sh prepared-release\.json/);
  assert.doesNotMatch(backendDeploy, /backend-release-\$\{\{ inputs\.source_sha \}\}/);
  assert.match(preparation, /selected-release-\$\{\{ inputs\.source_sha \}\}/);
  assert.match(preparation.split('\njobs:')[0], /actions: read/);
  const cliDeploy = read('.github/workflows/publish-cli.yml');
  assert.match(cliDeploy, /cp cli\/railway\.json \.railway-upload\/railway\.json/);
  assert.doesNotMatch(cliDeploy, /cargo build/);
});

test('Node workflows cache pnpm and browser downloads by the web lockfile', () => {
  const integrationCi = read('.github/workflows/scope-integration-ci.yml');
  for (const workflow of [integrationCi, read('.github/workflows/rust-workspace-checks.yml'), read('.github/workflows/scope-web-ci.yml')]) {
    assert.match(workflow, /uses: pnpm\/action-setup@[0-9a-f]{40} # v5/);
    assert.match(workflow, /cache: pnpm/);
    assert.match(workflow, /cache-dependency-path: web\/pnpm-lock\.yaml/);
  }
  assert.match(integrationCi, /path: ~\/\.cache\/ms-playwright/);
  assert.match(integrationCi, /key: playwright-\$\{\{ runner\.os \}\}-\$\{\{ hashFiles\('web\/pnpm-lock\.yaml'\) \}\}/);
});

test('production success follows the complete monitored transition', () => {
  for (const workflow of [read('.github/workflows/deploy-backend.yml'), read('.github/workflows/deploy-web.yml')]) {
    const recordStep = workflow.slice(workflow.indexOf('      - name: Record successful Railway'));
    assert.match(recordStep, /if: steps\.transition\.outcome == 'success'/);
    assert.match(workflow, /name: Deploy to Railway\n\s+id: transition/);
  }
});

test('release selection uses the trusted control revision before exposing a source revision', () => {
  const release = read('.github/workflows/release.yml');
  const requireMain = release.indexOf('- name: Require main for releases');
  const selection = release.indexOf('run: node .github/scripts/release-selection.mjs');
  const retain = release.indexOf('- name: Retain selected immutable release');
  assert(requireMain >= 0 && selection > requireMain && retain > selection);
  assert.match(release.slice(requireMain, selection), /test "\$GITHUB_REF" = refs\/heads\/main/);
  assert.match(read('.github/workflows/deploy-backend.yml'), /ref: \$\{\{ github\.sha \}\}\n\s+persist-credentials: false/);
  assert.match(release.split('\njobs:')[0], /deployments: read/);
  assert.doesNotMatch(release.split('\njobs:')[0], /: write/);
  const checks = read('.github/workflows/scope-checks-image.yml');
  const candidate = checks.slice(checks.indexOf('  validate:'), checks.indexOf('  build:'));
  assert.match(candidate, /if: github\.event_name == 'pull_request'/);
  assert.doesNotMatch(candidate, /: write/);
  assert.match(checks.slice(checks.indexOf('  build:')), /if: github\.event_name != 'pull_request'/);
});

test('CI is pull-request-only and Release is scheduled/manual with a shared check owner', () => {
  const ci = read('.github/workflows/ci.yml');
  const release = read('.github/workflows/release.yml');
  assert.match(ci, /  pull_request:/);
  assert.doesNotMatch(ci.split('\nconcurrency:')[0], /schedule:|workflow_dispatch:|push:/);
  const triggers = release.split('\nconcurrency:')[0];
  assert.match(triggers, /cron: "8,38 \* \* \* \*"/);
  assert.match(triggers, /workflow_dispatch:/);
  assert.doesNotMatch(triggers, /pull_request:|push:/);
  for (const caller of [ci, release]) assert.match(caller, /uses: \.\/\.github\/workflows\/validate.yml/);
});
