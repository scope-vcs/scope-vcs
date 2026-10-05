import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, cpSync, mkdtempSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, resolve } from 'node:path';
import test from 'node:test';
import { classifyChanges } from './plan-production-deployment.mjs';

const root = resolve(import.meta.dirname, '../..');
const read = (path) => readFileSync(resolve(root, path), 'utf8');
const manifest = JSON.parse(read('.github/deployment-services.json'));
const gates = ['backend', 'cli', 'cli-bundle', 'web', 'contract', 'policy', 'integration', 'ops', 'dependency-analyzer'];

function commands(gate, ...args) {
  const dir = mkdtempSync(resolve(tmpdir(), 'scope-gates-'));
  try {
    for (const tool of ['cargo', 'npm', 'pnpm', 'node', 'bash', 'python3']) {
      writeFileSync(resolve(dir, tool), '#!/bin/sh\ncase "$1" in *dev/checks/*) exec /bin/bash "$@" ;; esac\nprintf "%s" "$(basename "$0")"\nprintf " %s" "$@"\nprintf "\\n"\n', { mode: 0o755 });
    }
    writeFileSync(resolve(dir, 'rustc'), '#!/bin/sh\nprintf "host: x86_64-unknown-linux-gnu\\n"\n', { mode: 0o755 });
    return execFileSync('/bin/bash', [resolve(root, `dev/checks/${gate}`), ...args], {
      env: { ...process.env, PATH: `${dir}:${process.env.PATH}` }, encoding: 'utf8',
    }).trim().split('\n');
  } finally { rmSync(dir, { recursive: true, force: true }); }
}

test('the backend gate covers the whole workspace and the API feature suites explicitly', () => {
  const backend = commands('backend');
  assert.ok(backend.includes('cargo deny --locked check advisories'));
  assert.ok(backend.indexOf('cargo deny --locked check advisories') < backend.indexOf('cargo test --workspace --locked'));
  assert.ok(backend.includes('cargo test --workspace --locked'));
  assert.ok(backend.includes('cargo test -p api --features local-dev --locked dev::'));
  assert.ok(backend.includes('cargo test -p api --features smoke-seed --locked --lib smoke_seed::tests'));
});

test('the backend gate bounds PostgreSQL test concurrency on larger hosts', () => {
  const dir = mkdtempSync(resolve(tmpdir(), 'scope-test-concurrency-'));
  try {
    const receipt = resolve(dir, 'threads');
    writeFileSync(resolve(dir, 'cargo'), '#!/bin/sh\nif [ "$1" = test ]; then printf "%s\\n" "$RUST_TEST_THREADS" >> "$SCOPE_TEST_THREAD_RECEIPT"; fi\n', { mode: 0o755 });
    execFileSync('/bin/bash', [resolve(root, 'dev/checks/backend')], {
      env: { ...process.env, PATH: `${dir}:${process.env.PATH}`, RUST_TEST_THREADS: '128', SCOPE_TEST_THREAD_RECEIPT: receipt },
    });
    const threads = readFileSync(receipt, 'utf8').trim().split('\n');
    assert.ok(threads.length > 0);
    for (const count of threads) assert.equal(count, '4');
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

test('web gate includes resource, Hooks, convention and advisory checks; backend owns the contract; CLI and integration retain their coverage', () => {
  assert.deepEqual(commands('web'), [
    'pnpm test', 'pnpm check', 'pnpm build',
  ]);
  const webChecks = JSON.parse(read('web/package.json')).scripts.check;
  assert.equal(webChecks, 'pnpm typecheck && pnpm check:resource-boundary && pnpm check:hooks && pnpm check:conventions && pnpm check:advisories');
  assert.deepEqual(commands('contract'), ['pnpm check:api-contract']);
  const cliCommands = commands('cli');
  assert.ok(cliCommands.includes('cargo fmt --manifest-path cli/Cargo.toml -- --check'));
  assert.ok(cliCommands.includes('cargo deny --locked --manifest-path cli/Cargo.toml --config cli/deny.toml check advisories'));
  assert.ok(cliCommands.includes('cargo test --manifest-path cli/Cargo.toml --locked'));
  assert.ok(cliCommands.includes('cargo clippy --manifest-path cli/Cargo.toml --all-targets --locked -- -D warnings'));
  assert.ok(!cliCommands.some((command) => command.includes('--release')), cliCommands.join('\n'));
  const bundleCommands = commands('cli-bundle');
  assert.ok(bundleCommands.includes('cargo build --manifest-path cli/Cargo.toml --release --locked --bin scope --bin scope-cli-service'));
  assert.ok(bundleCommands.includes('npm ci --prefix dependency-analyzer --ignore-scripts'));
  assert.ok(bundleCommands.includes('bash cli/distribution/package-bundle.sh'));
  assert.ok(bundleCommands.includes('bash cli/distribution/verify-bundled-analyzer.sh'));
  assert.ok(bundleCommands.includes('node --test cli/distribution/install-smoke.test.mjs'));
  assert.deepEqual(commands('integration', 'cli'), ['cargo test --manifest-path cli/Cargo.toml --test contribution_flow --locked -- --ignored --nocapture']);
  assert.deepEqual(commands('integration', 'web'), ['pnpm test:smoke']);
  assert.deepEqual(commands('integration', 'journey'), ['cargo build --manifest-path cli/Cargo.toml --locked --bin scope', 'pnpm test:journey']);
  assert.deepEqual(commands('dependency-analyzer'), [
    'npm ci --ignore-scripts', 'npm test',
  ]);
});

test('local and GitHub callers use the shared inventory', () => {
  const github = ['rust-workspace-checks', 'scope-api-ci', 'scope-cli-build', 'scope-web-ci', 'ci', 'release', 'scope-integration-ci']
    .map((name) => read(`.github/workflows/${name}.yml`)).join('\n');
  for (const gate of gates) assert.ok(github.includes(`dev/checks/${gate}`), `GitHub: ${gate}`);
  for (const gate of ['policy', 'contract']) assert.ok(read('dev/check').includes(`dev/checks/${gate}`), `local: ${gate}`);
  assert.doesNotMatch(read('web/package.json'), /dev\/checks\/contract/);
  assert.doesNotMatch(read('.github/workflows/scope-web-ci.yml'), /rust-toolchain|rust-cache/);
});

const testsOutsideSharedGates = new Map([
  ['deploy/aws/recovery/tests/test_recovery.py', '.github/workflows/recovery-execute.yml'],
  ['deploy/railway/test-runtime-containers.sh', null],
]);

function runsTest(command, path) {
  const args = command.split(' ');
  if (args.includes('discover')) {
    const option = (flag, fallback) => (args.includes(flag) ? args[args.indexOf(flag) + 1] : fallback);
    const pattern = new RegExp(`^${option('-p', 'test*.py').replaceAll('.', '\\.').replaceAll('*', '.*')}$`);
    const start = option('-s', '.');
    return (dirname(path) === start || dirname(path).startsWith(`${start}/`)) && pattern.test(basename(path));
  }
  return args.includes(path) && !['-n', '--check', 'py_compile'].some((flag) => args.includes(flag));
}

test('every deployment, benchmark, and developer tooling test is run by a shared gate', () => {
  const invoked = ['ops', 'policy', 'cli'].flatMap((gate) => commands(gate));
  const tests = execFileSync('git', ['ls-files', '--cached', '--others', '--exclude-standard', '.github/scripts', 'bench', 'deploy', 'dev'], { cwd: root, encoding: 'utf8' })
    .trim().split('\n')
    .filter((path) => /(\.test\.(mjs|py)|\/test_[^/]*\.py|\/test-[^/]*\.(sh|py))$/.test(path)
      && existsSync(resolve(root, path)) && !testsOutsideSharedGates.has(path));
  for (const path of tests) {
    assert.ok(invoked.some((command) => runsTest(command, path)), `${path} has no test gate`);
  }
  for (const [path, workflow] of testsOutsideSharedGates) {
    if (workflow === null) continue;
    const runs = read(workflow).split('\n').some((line) => runsTest(line.trim(), path));
    assert.ok(runs, `${workflow} must run ${path}`);
  }
});

const alwaysOnGateInputs = [
  /^deploy\/railway\/(maintenance\.Dockerfile|test-runtime-containers\.sh)$/,
  /^deploy\/railway\/(ssh-bin\/ssh|ssh_known_hosts)$/,
  /^bench\//, /^deploy\/(aws|postgres|automation)\//, /^dev\/analytics\//, /^dev\/legal\//, /^dev\/licensing\//,
  /^dev\/checks\/(ops|policy|README\.md)$/, /^dev\/(check|test_local_process\.py|install-test-postgres\.sh)$/,
  /^\.github\/(source-size-audit|railway-experiments)\.json$/,
  /^\.github\/workflows\/(audit-railway-experiments|scope-aws-infrastructure(?:-execute)?|backup-monitor(?:-execute)?|recovery(?:-execute)?|deployment-tests|deployment-watcher-heartbeat|maintenance-runtime)\.yml$/,
  /^\.github\/scripts\/fixtures\//, /\.test\.mjs$/, /\.md$/,
];

function scriptsCoveredByAlwaysOnGates() {
  const commandText = ['ops', 'policy'].flatMap((gate) => commands(gate)).join('\n');
  const scripts = readdirSync(resolve(root, '.github/scripts'))
    .filter((name) => !name.endsWith('.test.mjs') && name !== 'fixtures');
  const covered = new Set(scripts.filter((name) => (
    commandText.includes(`.github/scripts/${name}`)
    || commandText.includes(`.github/scripts/${name.replace(/\.(mjs|sh|py)$/, '.test.mjs')}`)
    || commandText.includes(`.github/scripts/test-${name}`)
  )));
  for (let grew = true; grew;) {
    grew = false;
    for (const name of scripts) {
      if (covered.has(name)) continue;
      const loaders = [...covered, ...readdirSync(resolve(root, '.github/scripts')).filter((test) => test.endsWith('.test.mjs'))];
      if (loaders.some((loader) => read(`.github/scripts/${loader}`).includes(name))) {
        covered.add(name);
        grew = true;
      }
    }
  }
  return new Set([...covered].map((name) => `.github/scripts/${name}`));
}

test('gate inputs select a lane unless the always-on gates own them', () => {
  const paths = execFileSync('git', ['ls-files', '--cached', '--others', '--exclude-standard'], { cwd: root, encoding: 'utf8' }).trim().split('\n');
  const gateInputs = paths.filter((path) => existsSync(resolve(root, path)) && /^(dev\/|\.github\/(scripts|workflows)\/|bench\/|deploy\/)/.test(path));
  gateInputs.push('.github/source-size-audit.json', '.github/railway-experiments.json');
  const coveredScripts = scriptsCoveredByAlwaysOnGates();
  for (const path of gateInputs) {
    const selected = Object.values(classifyChanges(manifest, [path])).some(Boolean);
    const alwaysOn = coveredScripts.has(path) || alwaysOnGateInputs.some((pattern) => pattern.test(path));
    assert.ok(selected || alwaysOn, `${path} must select a lane or be run, tested, or loaded by the operations or policy gates`);
  }
  for (const gate of ['ops', 'policy']) {
    for (const caller of ['ci', 'release']) assert.ok(read(`.github/workflows/${caller}.yml`).includes(`dev/checks/${gate}`), `${caller} must always run ${gate}`);
  }
  for (const caller of ['ci', 'release']) {
    const ops = read(`.github/workflows/${caller}.yml`).split('\n  ops:\n')[1].split(/\n  [\w-]+:\n/)[0];
    assert.match(ops, /SCOPE_REQUIRE_POSTGRES_CLUSTER: '1'\n\s+run: \.\/dev\/checks\/ops\n/, `${caller} ops must require the PostgreSQL cluster tests`);
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

test('the CLI is built once per target and the matrix owns the deployed service artifact', () => {
  const workflow = read('.github/workflows/scope-cli-build.yml');
  const jobsSection = workflow.slice(workflow.indexOf('\njobs:\n'));
  assert.deepEqual(
    [...jobsSection.matchAll(/^  ([\w-]+):$/gm)].map(([, name]) => name),
    ['prepare', 'checks', 'build'],
  );
  const checks = jobsSection.slice(jobsSection.indexOf('\n  checks:\n'), jobsSection.indexOf('\n  build:\n'));
  const build = jobsSection.slice(jobsSection.indexOf('\n  build:\n'));
  assert.doesNotMatch(checks, /--release/);
  assert.match(checks, /run: \.\/dev\/checks\/cli\n/);
  assert.match(checks, /if: \$\{\{ !inputs\.validate_targets \}\}\n\s+run: \.\/dev\/checks\/cli-bundle\n/);
  assert.match(checks, /if: inputs\.validate_service_release && !inputs\.validate_targets\n/);
  const serviceGate = "if: inputs.validate_service_release && matrix.target == 'x86_64-unknown-linux-gnu'";
  assert.equal(build.split(serviceGate).length - 1, 2);
  const upload = build.slice(build.indexOf('      - name: Upload service'));
  assert.match(upload, /name: cli-service-release-\$\{\{ github\.sha \}\}/);
  assert.match(upload, /path: artifacts\/scope-cli-service\.tar\.gz/);
  const pack = build.slice(build.indexOf('      - name: Pack service'), build.indexOf('      - name: Upload service'));
  assert.match(pack, /release=cli\/target\/\$\{\{ matrix\.target \}\}\/release/);
  assert.match(pack, /--file artifacts\/scope-cli-service\.tar\.gz/);
  assert.match(pack, /--directory "\$release"/);
  assert.match(pack, /\n\s+scope-cli-service LICENSE NOTICE third-party-rust\.txt\n/);
  const publish = read('.github/workflows/publish-cli.yml');
  assert.match(publish, /name: cli-service-release-\$\{\{ inputs\.source_sha \|\| github\.sha \}\}/);
  assert.match(publish, /--file artifacts\/scope-cli-service\.tar\.gz[\s\\]+--directory \.railway-cli\/bin/);
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

test('bundle packaging rejects missing and stale release identities after all builds', () => {
  const workflow = read('.github/workflows/scope-cli-build.yml');
  const identityStep = workflow.split('      - name: Verify release identity\n')[1]
    .split('      - name: Package CLI and managed analyzer runtime\n')[0];
  const script = identityStep.split('        run: |\n')[1].replace(/^          /gm, '');
  assert.ok(
    workflow.indexOf('      - name: Build native installer service\n')
      < workflow.indexOf('      - name: Verify release identity\n'),
    'identity must be checked after Cargo finishes building release binaries',
  );
  assert.ok(
    workflow.indexOf('      - name: Verify release identity\n')
      < workflow.indexOf('      - name: Package CLI and managed analyzer runtime\n'),
    'identity must be checked before the binary enters the bundle',
  );
  const sha = '1234567890abcdef1234567890abcdef12345678';
  const dir = mkdtempSync(resolve(tmpdir(), 'scope-release-identity-'));
  try {
    const run = (smoke, output, embedded = output, buildSha = sha) => {
      writeFileSync(resolve(dir, 'scope'), `#!/bin/sh\n# ${embedded}\nprintf '%s\\n' '${output}'\n`, { mode: 0o755 });
      return spawnSync('bash', ['-e', '-o', 'pipefail', '-c', script
        .replaceAll('${{ matrix.binary }}', 'scope')
        .replaceAll('${{ matrix.smoke }}', String(smoke))], {
        cwd: dir, env: { ...process.env, SCOPE_BUILD_SHA: buildSha }, encoding: 'utf8',
      });
    };
    const current = `scope 0.1.0 (build ${sha}; protocol 1)`;
    for (const smoke of [true, false]) {
      assert.equal(run(smoke, current).status, 0);
      assert.notEqual(run(smoke, 'scope 0.1.0 (build development; protocol 1)').status, 0);
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

function releasePath(selected, { reuse = false, resumeStaging = false, failure = '', cancelled = false, ref = 'refs/heads/main', backendActivatesWeb = false, replaceRedeployedComponent = '' } = {}) {
  const jobs = releaseJobs();
  const outputs = Object.fromEntries(['runner_image', 'cache', 'worker', 'media_worker', 'router', 'media', 'api', 'web', 'cli']
    .map((key) => [key, String(selected.includes(key))]));
  outputs.backend_selected = String(['cache', 'worker', 'media_worker', 'router', 'media', 'api']
    .some((key) => outputs[key] === 'true'));
  outputs.prepared_run_id = reuse || resumeStaging ? '123' : '';
  outputs.recover_cutover_id = reuse ? '456' : '';
  outputs.resume_staging = String(resumeStaging);
  const needs = { plan: { result: 'success', outputs }, ...Object.fromEntries(['policy', 'ops', 'validation', 'server-validation'].map(key => [key, { result: key === failure ? 'failure' : 'success' }])) };
  for (const key of ['readiness-preflight', 'smoke-tools', 'release-preparation', 'production-preflight', 'staging', 'backend-deploy', 'web-deploy', 'cli-deploy', 'production-health-gate']) {
    const expression = jobs[key].if.replace(/needs\.([\w-]+)/g, 'needs["$1"]');
    const enabled = Function('needs', 'github', 'cancelled', 'inputs', `return (${expression});`)(needs, { ref }, () => cancelled,
      { replace_redeployed_component: replaceRedeployedComponent });
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
  const runner = releasePath(['runner_image']);
  assert.equal(runner.staging.result, 'skipped');
  assert.equal(runner['production-health-gate'].result, 'success');
  for (const selected of [[], ['cli'], ['runner_image']]) {
    const replacement = releasePath(selected, { replaceRedeployedComponent: 'api', failure: 'readiness-preflight' });
    assert.equal(replacement['readiness-preflight'].result, 'failure', 'explicit replacements must reach preflight');
    assert.equal(replacement['production-health-gate'].result, 'skipped');
    assert.equal(replacement['cli-deploy'].result, 'skipped');
  }
});

test('failed preflight, staging or activation cannot publish a successful release', () => {
  for (const failure of ['policy', 'ops', 'validation', 'server-validation', 'readiness-preflight', 'smoke-tools', 'production-preflight', 'staging', 'backend-deploy', 'web-deploy', 'cli-deploy']) {
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
  assert.match(preparation, /prepare-release-images\.sh/);
  assert.match(read('.github/scripts/prepare-release-images.sh'), /prepare-railway-artifact\.sh/);
  assert.match(read('.github/workflows/scope-api-ci.yml'), /name: backend-release-\$\{\{ github\.sha \}\}/);
  assert.match(backendDeploy, /extract-railway-maintenance\.sh prepared-release\.json/);
  assert.doesNotMatch(backendDeploy, /backend-release-\$\{\{ inputs\.source_sha \}\}/);
  assert.match(preparation, /selected-release-\$\{\{ inputs\.source_sha \}\}/);
  assert.match(preparation.split('\njobs:')[0], /actions: read/);
  const cliDeploy = read('.github/workflows/publish-cli.yml');
  assert.match(cliDeploy, /SCOPE_PREPARED_RELEASE_PATH: prepared-cli-release\.json/);
  assert.match(cliDeploy, /prepare-railway-artifact\.sh cli-downloads \.railway-cli prepared-cli-release\.json/);
  assert.doesNotMatch(cliDeploy, /cargo build/);
});

test('CLI publication configures the advertised installer origin before deployment', (t) => {
  const workflow = read('.github/workflows/publish-cli.yml');
  const block = workflow.match(/      - name: Deploy artifacts to Railway\n[\s\S]*?        run: \|\n((?:          .*\n)+)/)?.[1];
  assert.ok(block, 'CLI deployment shell step must be present');
  const dir = mkdtempSync(resolve(tmpdir(), 'scope-cli-publication-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  mkdirSync(resolve(dir, '.github/scripts'), { recursive: true });
  mkdirSync(resolve(dir, 'bin'));
  writeFileSync(resolve(dir, '.github/deployment-services.json'), JSON.stringify(manifest));
  writeFileSync(resolve(dir, 'bin/railway'), '#!/bin/sh\nprintf "%s\\n" "$@" >> events\n', { mode: 0o755 });
  writeFileSync(resolve(dir, '.github/scripts/deploy-railway.sh'), '#!/bin/sh\nprintf "deploy %s\\n" "$1" >> events\n');
  const result = spawnSync('/bin/bash', ['--noprofile', '--norc', '-euo', 'pipefail', '-c', block.replace(/^          /gm, '')], {
    cwd: dir,
    encoding: 'utf8',
    env: { PATH: `${resolve(dir, 'bin')}:${process.env.PATH}` },
  });
  assert.equal(result.status, 0, result.stderr);
  const events = readFileSync(resolve(dir, 'events'), 'utf8').trim().split('\n');
  assert.deepEqual(events, [
    'variable', 'set', '--project', manifest.railway.projectId,
    '--environment', manifest.environments.production.environmentId,
    '--service', manifest.services['cli-downloads'].id, '--skip-deploys',
    `SCOPE_CLI_PUBLIC_URL=${manifest.environments.production.cliPublicOrigin}`,
    `deploy ${manifest.services['cli-downloads'].id}`,
  ]);
});

test('staging verifies pinned Git before credentials or deployment mutations', () => {
  const workflow = read('.github/workflows/deploy-staging.yml');
  const install = workflow.indexOf('- name: Install reviewed Git for staging smoke');
  const verify = workflow.indexOf('- name: Verify staging Git version');
  const credentials = workflow.indexOf('- name: Create staging-scoped Railway token');
  assert.ok(install > workflow.indexOf('- name: Checkout trusted orchestration'));
  assert.ok(verify > install && credentials > verify);
  const setup = workflow.slice(install, credentials);
  assert.match(setup, /jq -er '\.git\.version' dev\/tool-versions\.json/);
  assert.match(setup, /jq -er '\.git\.sourceSha256' dev\/tool-versions\.json/);
  assert.match(setup, /sudo bash deploy\/railway\/install-git\.sh "\$version" "\$source_sha256"/);
  assert.match(setup, /echo \/opt\/git\/bin >> "\$GITHUB_PATH"/);
  assert.match(setup, /run: node dev\/check-git-version\.mjs/);
  assert.doesNotMatch(setup, /\n\s+(?:if:|continue-on-error:)/);
});

test('every external action and reusable workflow is pinned to a full commit SHA', () => {
  const unpinned = readdirSync(resolve(root, '.github/workflows')).filter((name) => /\.ya?ml$/.test(name)).flatMap((name) => (
    [...read(`.github/workflows/${name}`).matchAll(/^\s*(?:-\s+)?['"]?uses['"]?\s*:\s*['"]?([^\s'"#]+)/gm)]
      .map(([, action]) => action)
      .filter((action) => !action.startsWith('./') && !/@[0-9a-f]{40}$/.test(action))
      .map((action) => `${name}: ${action}`)
  ));
  assert.deepEqual(unpinned, []);
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
  const runner = read('.github/workflows/scope-runner-image.yml');
  const candidate = runner.slice(runner.indexOf('\n  validate:\n'), runner.indexOf('\n  publish:\n'));
  assert.match(candidate, /if: \$\{\{ !inputs\.publish \}\}/);
  assert.doesNotMatch(candidate, /: write/);
  const publish = runner.slice(runner.indexOf('\n  publish:\n'));
  assert.match(publish, /if: inputs\.publish\n/);
  assert.match(publish, /- name: Require main for publication\n\s+run: test "\$GITHUB_REF" = refs\/heads\/main\n/);
});

test('runner base images pass the container lifecycle gate and scan before publication', () => {
  const runner = read('.github/workflows/scope-runner-image.yml');
  const candidate = runner.slice(runner.indexOf('\n  validate:\n'), runner.indexOf('\n  publish:\n'));
  const publish = runner.slice(runner.indexOf('\n  publish:\n'));
  for (const lane of [candidate, publish]) {
    assert.match(lane, /file: runner-runtime\/Dockerfile\n/);
    assert.match(lane, /push: false\n\s+load: true/);
    assert.match(lane, /run: dev\/checks\/runner-runtime-container --image /);
    assert.match(lane, /run: \.github\/scripts\/scan-image\.sh docker /);
  }
  const verify = publish.indexOf('- name: Verify runtime process lifecycle');
  const scan = publish.indexOf('- name: Scan verified image before publishing');
  const push = publish.indexOf('- name: Publish verified image');
  assert(verify >= 0 && scan > verify && push > scan);
  assert.match(publish.slice(push), /bash \.github\/scripts\/push-image-tags\.sh <<< "\$IMAGE"/);
});

test('CI runs on pull requests and Scope request branches, and Release dispatch is owned by the watcher', () => {
  const ci = read('.github/workflows/ci.yml');
  const release = read('.github/workflows/release.yml');
  const ciTriggers = ci.split('\nconcurrency:')[0];
  assert.match(ciTriggers, /  pull_request:\n  push:\n    branches: \['scope\/\*\*'\]\n/);
  assert.doesNotMatch(ciTriggers, /schedule:|workflow_dispatch:/);
  assert.match(ci, /group: scope-ci-\$\{\{ github\.event\.pull_request\.number \|\| github\.ref \}\}/);
  assert.match(ci, /base="\$\(git merge-base origin\/main HEAD\)"/);
  assert.match(ci, /BASE_SHA: \$\{\{ steps\.base\.outputs\.sha \}\}/);
  assert.doesNotMatch(ci, /github\.event\.before/);
  const triggers = release.split('\nconcurrency:')[0];
  assert.doesNotMatch(triggers, /schedule:|cron:/);
  assert.match(triggers, /schedule_intent:/);
  assert.match(triggers, /workflow_dispatch:/);
  assert.doesNotMatch(triggers, /pull_request:|push:/);
  for (const caller of [ci, release]) assert.match(caller, /uses: \.\/\.github\/workflows\/validate.yml/);
});

function stepScript(job, name) {
  const step = job.slice(job.indexOf(`      - name: ${name}\n`));
  const script = step.match(/        run: \|\n((?:          .*\n)+)/)?.[1];
  assert.ok(script, `${name} must execute its assertion`);
  return script.replace(/^          /gm, '');
}

const gateScript = (job) => stepScript(job, 'Require successful planning and validation');
const validationGateScript = (job) => stepScript(job, 'Require selected validations to succeed');

test('required PR check runs after failures and rejects every unsuccessful prerequisite', () => {
  const ci = read('.github/workflows/ci.yml');
  const job = ci.slice(ci.indexOf('\n  required-pr-checks:\n'));
  assert.match(job, /name: Required PR checks\n/);
  assert.match(job, /needs: \[pull-request, plan, policy, ops, validation\]\n/);
  assert.match(job, /if: \$\{\{ always\(\) \}\}\n/);
  assert.match(job, /PLAN_RESULT: \$\{\{ needs.plan.result \}\}/);
  assert.match(job, /VALIDATION_RESULT: \$\{\{ needs.validation.result \}\}/);
  for (const plan of ['success', 'failure', 'cancelled', 'skipped', '']) {
    for (const validation of ['success', 'failure', 'cancelled', 'skipped', '']) {
      const result = spawnSync('bash', ['-e', '-o', 'pipefail', '-c', gateScript(job)], {
        env: { ...process.env, PLAN_RESULT: plan, VALIDATION_RESULT: validation, POLICY_RESULT: 'success', OPS_RESULT: 'success' },
        encoding: 'utf8',
      });
      assert.equal(result.status === 0, plan === 'success' && validation === 'success', `${plan}/${validation}`);
    }
  }
});

test('required PR gate rejects failed policy and operations even when validation passes', () => {
  const ci = read('.github/workflows/ci.yml');
  const script = gateScript(ci.slice(ci.indexOf('\n  required-pr-checks:\n')));
  for (const key of ['POLICY_RESULT', 'OPS_RESULT']) {
    for (const failure of ['failure', 'cancelled', 'skipped', '']) {
      const result = spawnSync('bash', ['-e', '-c', script], {
        env: { ...process.env, PLAN_RESULT: 'success', VALIDATION_RESULT: 'success', POLICY_RESULT: 'success', OPS_RESULT: 'success', [key]: failure },
      });
      assert.notEqual(result.status, 0, `${key}: ${failure}`);
    }
  }
});

test("a Scope request revision that an open pull request tests reports that pull request's result", () => {
  const ci = read('.github/workflows/ci.yml');
  const jobs = ci.slice(ci.indexOf('\njobs:\n'));
  assert.match(jobs, /  pull-request:\n(?:    .*\n)*?    if: \$\{\{ github\.event_name == 'push' \}\}\n/);
  for (const name of ['plan', 'policy', 'ops']) {
    assert.match(jobs, new RegExp(`  ${name}:\\n(?:    .*\\n)*?    needs: pull-request\\n    if: \\$\\{\\{ !cancelled\\(\\) && needs\\.pull-request\\.outputs\\.number == '' \\}\\}\\n`));
  }
  assert.match(jobs, /  validation:\n(?:    .*\n)*?    if: \$\{\{ !cancelled\(\) && needs\.plan\.result == 'success' \}\}\n/);
  const gate = ci.slice(ci.indexOf('\n  required-pr-checks:\n'));
  assert.match(gate, /- name: Report the pull request's result\n        if: \$\{\{ needs\.pull-request\.outputs\.number != '' \}\}\n/);
  assert.match(gate, /- name: Require successful planning and validation\n        if: \$\{\{ needs\.pull-request\.outputs\.number == '' \}\}\n/);
  const script = stepScript(gate, "Report the pull request's result");
  const report = (answers) => {
    const dir = mkdtempSync(resolve(tmpdir(), 'scope-ci-report-'));
    try {
      writeFileSync(resolve(dir, 'answers'), answers.join('\n') + '\n');
      writeFileSync(resolve(dir, 'gh'), `#!/bin/bash\nn=$(cat "${dir}/count" 2>/dev/null || echo 0); echo $((n + 1)) > "${dir}/count"\nline=$(sed -n "$((n + 1))p" "${dir}/answers")\n[[ "$line" == error ]] && exit 1\necho "$line"\n`, { mode: 0o755 });
      writeFileSync(resolve(dir, 'sleep'), '#!/bin/sh\n', { mode: 0o755 });
      return spawnSync('bash', ['-e', '-o', 'pipefail', '-c', script], {
        env: { ...process.env, PATH: `${dir}:${process.env.PATH}`, REPOSITORY: 'o/r', SHA: 'a'.repeat(40), PULL_REQUEST: '7' },
        encoding: 'utf8',
      }).status;
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  };
  assert.equal(report(['', 'error', 'u in_progress null', 'u completed success']), 0);
  for (const conclusion of ['failure', 'cancelled', 'timed_out', 'skipped']) {
    assert.notEqual(report([`u completed ${conclusion}`]), 0, conclusion);
  }
});

test('a Scope request revision reuses a pull request run only when that run tests the same tree', () => {
  const ci = read('.github/workflows/ci.yml');
  const script = stepScript(ci.slice(ci.indexOf('\n  pull-request:\n')), 'Find a pull request run that tests this commit');
  const find = ({ pulls = '7', behind = '0', runs = '123' }) => {
    const dir = mkdtempSync(resolve(tmpdir(), 'scope-ci-find-'));
    try {
      writeFileSync(resolve(dir, 'gh'), `#!/bin/bash\ncase "$2" in\n  */pulls) echo "${pulls}" ;;\n  */compare/*) echo "${behind}" ;;\n  */runs*) echo "${runs}" ;;\n  *) exit 1 ;;\nesac\n`, { mode: 0o755 });
      const output = resolve(dir, 'output');
      const result = spawnSync('bash', ['-e', '-o', 'pipefail', '-c', script], {
        env: { ...process.env, PATH: `${dir}:${process.env.PATH}`, REPOSITORY: 'o/r', SHA: 'a'.repeat(40), GITHUB_OUTPUT: output },
        encoding: 'utf8',
      });
      assert.equal(result.status, 0, result.stderr);
      return readFileSync(output, 'utf8');
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  };
  assert.equal(find({}), 'number=7\n');
  assert.equal(find({ pulls: '' }), 'number=\n');
  assert.equal(find({ behind: '2' }), 'number=\n', 'a head behind main was tested merged with code it lacks');
  assert.equal(find({ runs: '' }), 'number=\n', 'a pull request with conflicts never gets a run to wait for');
});

test('validation gate allows unselected jobs and reused artifacts but fails selected jobs and cancellation', () => {
  const workflow = read('.github/workflows/validate.yml');
  const job = workflow.slice(workflow.indexOf('\n  production-validation-gate:\n'));
  assert.match(job, /if: \$\{\{ always\(\) \}\}\n/);
  assert.match(job, /if: \$\{\{ cancelled\(\) \}\}\n\s+run: exit 1\n/);
  const expression = job.match(/VALIDATIONS_PASSED: >-\n\s*\$\{\{ ([\s\S]*?) \}\}/)?.[1];
  assert.ok(expression, 'validation predicate must be evaluated in the assertion, not the job condition');
  const evaluate = Function('inputs', 'needs', `return (${expression.replace(/needs\.([\w-]+)/g, 'needs["$1"]')});`);
  const selectedBy = {
    'runner-image': ['runner_image'],
    'server-validation': ['backend', 'web'],
    'cli-validation': ['cli'],
    'integration-validation': ['web', 'cli'],
  };
  for (const mask of Array.from({ length: 16 }, (_, i) => i)) {
    const inputs = Object.fromEntries(['runner_image', 'backend', 'web', 'cli'].map((key, index) => [key, String(Boolean(mask & (1 << index)))]));
    inputs.reuse_artifacts = false;
    inputs.validate_server = true;
    const needs = Object.fromEntries(Object.entries(selectedBy).map(([name, keys]) => [name, {
      result: keys.some((key) => inputs[key] === 'true') ? 'success' : 'skipped',
    }]));
    needs['server-validation'].result = 'success';
    assert.equal(evaluate(inputs, needs), true, `selection ${mask}`);
    for (const [name, state] of Object.entries(needs)) {
      if (state.result !== 'success') continue;
      for (const result of ['failure', 'cancelled', 'skipped']) {
        assert.equal(evaluate(inputs, { ...needs, [name]: { result } }), false, `${mask}: ${name}/${result}`);
      }
    }
    inputs.reuse_artifacts = true;
    const skipped = Object.fromEntries(Object.keys(needs).map((name) => [name, { result: 'skipped' }]));
    assert.equal(evaluate(inputs, skipped), true, `reused selection ${mask}`);
  }
  for (const value of ['true', 'false', '']) {
    const result = spawnSync('bash', ['-e', '-o', 'pipefail', '-c', validationGateScript(job)], {
      env: { ...process.env, VALIDATIONS_PASSED: value }, encoding: 'utf8',
    });
    assert.equal(result.status === 0, value === 'true', `assertion ${value}`);
  }
});

test('preparation can overlap CLI work while staging joins every required lane', () => {
  const release = read('.github/workflows/release.yml');
  const section = (name) => release.split(`\n  ${name}:\n`)[1].split(/\n  [\w-]+:\n/)[0];
  const needs = (name) => section(name).match(/needs: \[(.+)\]/)[1].split(', ');
  assert.deepEqual(needs('release-preparation'), ['plan', 'server-validation']);
  for (const required of ['plan', 'policy', 'ops', 'validation', 'release-preparation', 'readiness-preflight', 'production-preflight', 'smoke-tools']) {
    assert(needs('staging').includes(required), `staging must join ${required}`);
  }
  assert.doesNotMatch(section('plan'), /dev\/checks\/(policy|ops)/);
  for (const job of ['policy', 'ops']) assert.doesNotMatch(section(job), /needs:/);
});

test('server readiness rejects each failed selected build before packaging', () => {
  const workflow = read('.github/workflows/validate-server.yml');
  const expression = workflow.match(/VALIDATIONS_PASSED: >-\n\s*\$\{\{ ([\s\S]*?) \}\}/)[1];
  const evaluate = Function('inputs', 'needs', `return (${expression.replace(/needs\.([\w-]+)/g, 'needs["$1"]')});`);
  for (const backend of ['true', 'false']) for (const web of ['true', 'false']) {
    const inputs = { backend, web, reuse_artifacts: false };
    const needs = Object.fromEntries(['backend-validation', 'media-worker-image', 'web-validation'].map(key => [key, {
      result: (key === 'web-validation' ? web : backend) === 'true' ? 'success' : 'skipped',
    }]));
    assert.equal(evaluate(inputs, needs), true);
    for (const [job, { result }] of Object.entries(needs)) {
      if (result !== 'success') continue;
      for (const failure of ['failure', 'cancelled', 'skipped']) {
        assert.equal(evaluate(inputs, { ...needs, [job]: { result: failure } }), false, `${job}: ${failure}`);
      }
    }
  }
});
