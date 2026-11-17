import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { existsSync, cpSync, mkdtempSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, resolve } from 'node:path';
import test from 'node:test';
import { classifyChanges } from './plan-production-deployment.mjs';
import { checkMainAdvisories } from '../../.github/scripts/main-advisories.mjs';
import { backendPrebuiltComponents } from './deployment-components.mjs';

const root = resolve(import.meta.dirname, '../..');
const read = (path) => readFileSync(resolve(root, path), 'utf8');
const manifest = JSON.parse(read('.github/deployment-services.json'));
const gates = ['backend', 'cli', 'cli-bundle', 'web', 'web-components', 'contract', 'policy', 'integration', 'ops'];

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
  assert.ok(backend.includes('cargo deny --locked check bans sources'));
  assert.ok(backend.includes('cargo shear --locked --deny-warnings'));
  const [apiFilter] = backend.find((command) => command.startsWith('cargo test -p api --locked --lib -- ')).split(' ').slice(-1);
  assert.ok(backend.includes(`cargo test -p api --locked -- --skip ${apiFilter}`));
  assert.ok(backend.includes('cargo test -p api --features local-dev --locked dev::'));
  assert.ok(backend.includes('cargo test -p api --features smoke-seed --locked --lib smoke_seed::tests'));
});

test('releases and the daily main run check every lockfile for advisories, and request gates check none', () => {
  const daily = read('.github/workflows/main-advisories.yml');
  assert.match(daily, /schedule:\n    - cron: '[^']+ \* \* \*'/);
  assert.match(daily, /if: github.ref == 'refs\/heads\/main'/);
  const release = read('.github/workflows/validate.yml').split('\n  dependency-advisories:\n')[1].split('\n  production-validation-gate:\n')[0];
  assert.match(release, /if: \$\{\{ inputs\.release \}\}\n/);
  for (const workflow of [daily, release]) {
    assert.match(workflow, /run: \.\/dev\/install-cargo-tools\.sh/);
    assert.match(workflow, /uses: pnpm\/action-setup@/);
    assert.match(workflow, /run: node \.github\/scripts\/main-advisories\.mjs "\$RUNNER_TEMP\/[\w-]+\.json"/);
  }
  const checked = checkMainAdvisories((file, args) => ({
    status: 0, stdout: commands(basename(file), ...args).join('\n'),
  }));
  assert.deepEqual(checked.map(check => check.output.trim()), [
    'cargo deny --locked --format json check advisories',
    'cargo deny --locked --manifest-path cli/Cargo.toml --config cli/deny.toml --format json check advisories',
    'pnpm audit --audit-level high',
  ]);
  for (const gate of ['backend', 'cli', 'web']) {
    assert.deepEqual(commands(gate).filter(line => /advisories|audit/.test(line)), [], gate);
  }
  assert.doesNotMatch(JSON.stringify(JSON.parse(read('web/package.json')).scripts), /audit/);
});

test('releases and the daily main run scan the media worker image, and request validation does not', () => {
  const image = read('.github/workflows/scope-media-worker-image.yml');
  assert.match(image, /\n      scan:\n        type: boolean\n        default: false\n/);
  assert.match(image, /- name: Scan image and record codec SBOM\n        if: \$\{\{ inputs\.scan \}\}\n/);
  const callers = { 'validate-server': '${{ inputs.release }}', 'main-advisories': 'true', 'preview-build': null, 'warm-ci-caches': null };
  for (const [workflow, scan] of Object.entries(callers)) {
    const call = read(`.github/workflows/${workflow}.yml`).match(/uses: \.\/\.github\/workflows\/scope-media-worker-image\.yml\n    with:\n((?:      .*\n?)+)/)[1];
    assert.equal(call.match(/^      scan: (.*)$/m)?.[1] ?? null, scan, workflow);
  }
});

test('GitHub builds every prebuilt backend component exactly once across the deployment jobs', () => {
  const workflow = read('.github/workflows/scope-api-ci.yml');
  const built = [...workflow.matchAll(/\n            components: (.+)\n/g)].flatMap(([, components]) => components.split(' '));
  assert.deepEqual([...built].sort(), [...backendPrebuiltComponents()].sort());
});

test('GitHub runs every backend gate lane exactly once', () => {
  const workflow = read('.github/workflows/rust-workspace-checks.yml');
  const testLanes = workflow.match(/lane: \[([^\]]+)\]/)[1].split(', ');
  const lanes = [workflow.match(/dev\/checks\/backend (lint)$/m)[1], ...testLanes];
  assert.deepEqual(lanes.flatMap((lane) => commands('backend', lane)).sort(), commands('backend').sort());
  assert.match(workflow, /uses: \.\/\.github\/actions\/install-test-postgres/);
  assert.ok(workflow.indexOf('Install PostgreSQL test server') < workflow.indexOf('Test Rust workspace lane'));
  assert.doesNotMatch(workflow, /SCOPE_TEST_DATABASE_URL|services:/);
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

test('web gate includes resource, Hooks and convention checks; backend owns the contract; CLI and integration retain their coverage', () => {
  assert.deepEqual(commands('web'), [
    'pnpm test', 'pnpm check', 'pnpm build',
  ]);
  const webChecks = JSON.parse(read('web/package.json')).scripts.check;
  assert.equal(webChecks, 'pnpm typecheck && pnpm check:resource-boundary && pnpm check:lint && pnpm check:conventions');
  assert.deepEqual(commands('contract'), ['pnpm check:api-contract']);
  const cliCommands = commands('cli');
  assert.ok(cliCommands.includes('cargo fmt --manifest-path cli/Cargo.toml -- --check'));
  assert.ok(cliCommands.includes('cargo deny --locked --manifest-path cli/Cargo.toml --config cli/deny.toml check bans sources'));
  assert.ok(cliCommands.includes('cargo shear --locked --deny-warnings cli'));
  assert.ok(cliCommands.includes('cargo test --manifest-path cli/Cargo.toml --locked'));
  assert.ok(cliCommands.includes('cargo clippy --manifest-path cli/Cargo.toml --all-targets --locked -- -D warnings'));
  assert.ok(cliCommands.includes('cargo test -p scope-cli-downloads --locked'));
  assert.ok(!cliCommands.some((command) => command.includes('--release')), cliCommands.join('\n'));
  const bundleCommands = commands('cli-bundle');
  assert.ok(bundleCommands.includes('cargo build --manifest-path cli/Cargo.toml --profile release-check --locked --bin scope'));
  assert.ok(bundleCommands.includes('cargo build -p scope-cli-downloads --release --locked'));
  assert.ok(bundleCommands.includes('bash cli/distribution/package-bundle.sh'));
  assert.ok(bundleCommands.includes('node --test cli/distribution/install-smoke.test.mjs'));
  assert.deepEqual(commands('integration', 'cli'), ['cargo test --manifest-path cli/Cargo.toml --test contribution_flow --locked -- --ignored --nocapture']);
  assert.deepEqual(commands('integration', 'web'), ['pnpm test:smoke']);
  assert.deepEqual(commands('web-components'), ['pnpm test:components']);
  const scripts = JSON.parse(read('web/package.json')).scripts;
  assert.match(scripts['test:components'], /^pnpm check:brand && node --test "smoke\/components\/\*\.spec\.mjs"$/);
  assert.doesNotMatch(scripts['test:smoke'], /components/);
  assert.deepEqual(commands('integration', 'journey'), ['cargo build --manifest-path cli/Cargo.toml --locked --bin scope', 'pnpm test:journey']);
});

test('local and GitHub callers use the shared inventory', () => {
  const github = ['rust-workspace-checks', 'scope-api-ci', 'scope-cli-build', 'scope-web-ci', 'ci', 'release', 'scope-integration-ci']
    .map((name) => read(`.github/workflows/${name}.yml`)).join('\n');
  for (const gate of gates) assert.ok(github.includes(`dev/checks/${gate}`), `GitHub: ${gate}`);
  for (const gate of ['policy', 'contract']) assert.ok(read('dev/check').includes(`dev/checks/${gate}`), `local: ${gate}`);
  assert.doesNotMatch(read('web/package.json'), /dev\/checks\/contract/);
  assert.doesNotMatch(read('.github/workflows/scope-web-ci.yml'), /rust-toolchain|rust-cache/);
});

const publicGates = ['policy', 'backend', 'contract', 'cli', 'cli-bundle', 'web'];
const operationsInput = /(?:^|[\s"'=])(?:\.\/)?(?:deploy|bench)\/|\.github\/(?:deployment-services|railway-experiments|main-protection)\.json/;

test('the gates a checkout without deploy/ runs name no operations file', () => {
  for (const gate of publicGates) {
    for (const command of commands(gate)) assert.doesNotMatch(command, operationsInput, `${gate}: ${command}`);
  }
});

test('the local check skips operations only in a checkout without deploy/', (t) => {
  const dir = mkdtempSync(resolve(tmpdir(), 'scope-local-check-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  mkdirSync(resolve(dir, 'dev/checks'), { recursive: true });
  for (const file of ['check', 'dotenv.sh']) cpSync(resolve(root, 'dev', file), resolve(dir, 'dev', file));
  for (const gate of gates) writeFileSync(resolve(dir, 'dev/checks', gate), `#!/bin/sh\necho ${gate}\n`, { mode: 0o755 });
  const check = (...args) => spawnSync('/bin/bash', [resolve(dir, 'dev/check'), ...args], { encoding: 'utf8' });
  const partial = check();
  assert.equal(partial.status, 0, partial.stderr);
  assert.deepEqual(partial.stdout.trim().split('\n'), [...publicGates, 'Skipped operations checks: this checkout does not include deploy/.']);
  const refused = check('ops');
  assert.notEqual(refused.status, 0);
  assert.equal(refused.stdout, '');
  mkdirSync(resolve(dir, 'deploy/scripts'), { recursive: true });
  assert.deepEqual(check().stdout.trim().split('\n'), [...publicGates, 'ops']);
  assert.deepEqual(check('ops').stdout.trim().split('\n'), ['ops']);
});

const testsOutsideSharedGates = new Set(['deploy/railway/test-runtime-containers.sh']);

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
});

const alwaysOnGateInputs = [
  /^deploy\/railway\/(maintenance\.Dockerfile|test-runtime-containers\.sh)$/,
  /^deploy\/railway\/(ssh-bin\/ssh|ssh_known_hosts)$/,
  /^bench\//, /^deploy\/postgres\//, /^dev\/analytics\//, /^dev\/legal\//, /^dev\/licensing\//,
  /^dev\/checks\/(ops|policy)$/, /^dev\/(check|test_local_(process|stack|identity)\.py|install-test-postgres\.(sh|asc))$/,
  /^\.github\/(source-size-audit|railway-experiments)\.json$/,
  /^\.github\/workflows\/(audit-railway-experiments|daily-monitors|main-advisories|maintenance-runtime|preview-build|preview-environment(?:-cleanup|-signal)?)\.yml$/,
  /\.test\.mjs$/, /\.md$/,
];

const scriptDirectories = ['.github/scripts', 'deploy/scripts'];

function scriptsCoveredByAlwaysOnGates() {
  const commandText = ['ops', 'policy'].flatMap((gate) => commands(gate)).join('\n');
  const entries = scriptDirectories.flatMap((directory) => readdirSync(resolve(root, directory))
    .map((name) => ({ directory, name, path: `${directory}/${name}` })));
  const tests = entries.filter(({ name }) => name.endsWith('.test.mjs'));
  const scripts = entries.filter(({ name }) => !name.endsWith('.test.mjs'));
  const covered = new Set(scripts.filter(({ directory, name, path }) => (
    commandText.includes(path)
    || commandText.includes(`${directory}/${name.replace(/\.(mjs|sh|py)$/, '.test.mjs')}`)
    || commandText.includes(`${directory}/test-${name}`)
  )));
  for (let grew = true; grew;) {
    grew = false;
    for (const script of scripts) {
      if (covered.has(script)) continue;
      if ([...covered, ...tests].some((loader) => read(loader.path).includes(script.name))) {
        covered.add(script);
        grew = true;
      }
    }
  }
  return new Set([...covered].map(({ path }) => path));
}

test('gate inputs select a lane unless the always-on gates own them', () => {
  const paths = execFileSync('git', ['ls-files', '--cached', '--others', '--exclude-standard'], { cwd: root, encoding: 'utf8' }).trim().split('\n');
  const gateInputs = paths.filter((path) => existsSync(resolve(root, path)) && /^(dev\/|\.github\/(scripts|workflows)\/|bench\/|deploy\/)/.test(path));
  gateInputs.push('.github/source-size-audit.json', '.github/railway-experiments.json');
  const coveredScripts = scriptsCoveredByAlwaysOnGates();
  for (const path of gateInputs) {
    const selected = Object.values(classifyChanges(manifest, [path])).some(Boolean) || manifest.changeScopes.release.files.includes(path);
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
    for (const script of ['check-source-size.mjs', 'repository-files.mjs']) cpSync(resolve(root, '.github/scripts', script), resolve(dir, '.github/scripts', script));
    writeFileSync(resolve(dir, '.github/source-size-audit.json'), JSON.stringify({ version: 1, production: [] }));
    writeFileSync(resolve(dir, 'worker/src/oversized.rs'), '// outside web\n'.repeat(1001));
    execFileSync('git', ['init', '--quiet', dir]);
    const result = spawnSync(process.execPath, ['.github/scripts/check-source-size.mjs'], { cwd: dir, encoding: 'utf8' });
    assert.equal(result.status, 1);
    assert.match(result.stderr, /worker\/src\/oversized.rs/);
  } finally { rmSync(dir, { recursive: true, force: true }); }
});

test('the CLI checks avoid release builds and publication downloads the service artifact the build uploads', () => {
  const workflow = read('.github/workflows/scope-cli-build.yml');
  const checks = workflow.slice(workflow.indexOf('\n  checks:\n'), workflow.indexOf('\n  build:\n'));
  const upload = workflow.slice(workflow.indexOf('      - name: Upload service'));
  assert.doesNotMatch(checks, /--release/);
  assert.match(upload, /name: cli-service-release-\$\{\{ github\.sha \}\}/);
  assert.match(upload, /path: artifacts\/scope-cli-service\.tar\.gz/);
  const publish = read('.github/workflows/publish-cli.yml');
  assert.match(publish, /name: cli-service-release-\$\{\{ github\.sha \}\}/);
  assert.match(publish, /--file artifacts\/scope-cli-service\.tar\.gz[\s\\]+--directory \.railway-cli\/bin/);
});

test('bundle packaging rejects missing and stale release identities after all builds', () => {
  const workflow = read('.github/workflows/scope-cli-build.yml');
  const identityStep = workflow.split('      - name: Verify release identity\n')[1]
    .split('      - name: Package CLI\n')[0];
  const script = identityStep.split('        run: |\n')[1].replace(/^          /gm, '');
  const buildStep = workflow.split('      - name: Build scope and the native installer service\n')[1]
    .split('      - name: Verify release identity\n')[0];
  assert.match(
    buildStep,
    /cargo build -p scope-cli-downloads [^\n]*&\n[\s\S]*\bwait "\$service"\n/,
    'identity must be checked after Cargo finishes building release binaries',
  );
  assert.ok(
    workflow.indexOf('      - name: Verify release identity\n')
      < workflow.indexOf('      - name: Package CLI\n'),
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

function releasePath(selected, { failure = '', cancelled = false, ref = 'refs/heads/main' } = {}) {
  const jobs = releaseJobs();
  const outputs = Object.fromEntries(['backend_selected', 'web', 'cli'].map((key) => [key, String(selected.includes(key))]));
  const needs = { plan: { result: 'success', outputs }, ...Object.fromEntries(['policy', 'ops', 'validation', 'server-validation'].map(key => [key, { result: key === failure ? 'failure' : 'success' }])) };
  for (const key of ['release-preparation', 'production-deploy', 'cli-deploy']) {
    const expression = jobs[key].if.replace(/needs\.([\w-]+)/g, 'needs["$1"]');
    const enabled = Function('needs', 'github', 'cancelled', `return (${expression});`)(needs, { ref }, () => cancelled);
    needs[key] = { result: enabled ? key === failure ? 'failure' : 'success' : 'skipped' };
  }
  return needs;
}

test('a release deploys the selected application once and publishes the CLI only after it', () => {
  const empty = releasePath([]);
  for (const key of ['release-preparation', 'production-deploy', 'cli-deploy']) assert.equal(empty[key].result, 'skipped', key);
  for (const selected of [['web'], ['backend_selected'], ['backend_selected', 'web', 'cli']]) {
    const path = releasePath(selected);
    assert.equal(path['production-deploy'].result, 'success', selected.join());
    assert.equal(path['cli-deploy'].result, selected.includes('cli') ? 'success' : 'skipped', selected.join());
  }
  const cli = releasePath(['cli']);
  assert.equal(cli['production-deploy'].result, 'skipped');
  assert.equal(cli['cli-deploy'].result, 'success');
});

test('a failed check, build or deploy leaves production and the CLI unpublished', () => {
  for (const failure of ['policy', 'ops', 'validation', 'server-validation', 'release-preparation', 'production-deploy']) {
    const result = releasePath(['backend_selected', 'web', 'cli'], { failure });
    if (failure !== 'production-deploy') assert.equal(result['production-deploy'].result, 'skipped', failure);
    assert.equal(result['cli-deploy'].result, 'skipped', failure);
  }
  for (const options of [{ cancelled: true }, { ref: 'refs/heads/feature' }]) {
    const result = releasePath(['backend_selected', 'web', 'cli'], options);
    for (const key of ['release-preparation', 'production-deploy', 'cli-deploy']) assert.equal(result[key].result, 'skipped', key);
  }
});

test('production and CLI jobs deploy prepared images without building', () => {
  const release = read('.github/workflows/release.yml');
  const deploy = release.split('\n  production-deploy:\n')[1].split(/\n  [\w-]+:\n/)[0];
  assert.match(deploy, /name: prepared-release-\$\{\{ github\.sha \}\}/);
  assert.match(deploy, /SCOPE_PREPARED_RELEASE_PATH: artifacts\/prepared-release\.json/);
  assert.match(deploy, /persist-credentials: false/);
  assert.doesNotMatch(deploy, /cargo build|docker build|railway up|pnpm build|RAILWAY_API_TOKEN/);
  assert.match(read('.github/workflows/prepare-release.yml'), /name: prepared-release-\$\{\{ inputs\.source_sha \}\}/);
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
  mkdirSync(resolve(dir, 'deploy/scripts'), { recursive: true });
  mkdirSync(resolve(dir, '.github'));
  mkdirSync(resolve(dir, 'bin'));
  writeFileSync(resolve(dir, '.github/deployment-services.json'), JSON.stringify(manifest));
  writeFileSync(resolve(dir, 'bin/railway'), '#!/bin/sh\nprintf "%s\\n" "$@" >> events\n', { mode: 0o755 });
  writeFileSync(resolve(dir, 'deploy/scripts/deploy-railway.sh'), '#!/bin/sh\nprintf "deploy %s\\n" "$1" >> events\n');
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

test('every external action and reusable workflow is pinned to a full commit SHA', () => {
  const unpinned = readdirSync(resolve(root, '.github/workflows')).filter((name) => /\.ya?ml$/.test(name)).flatMap((name) => (
    [...read(`.github/workflows/${name}`).matchAll(/^\s*(?:-\s+)?['"]?uses['"]?\s*:\s*['"]?([^\s'"#]+)/gm)]
      .map(([, action]) => action)
      .filter((action) => !action.startsWith('./') && !/@[0-9a-f]{40}$/.test(action))
      .map((action) => `${name}: ${action}`)
  ));
  assert.deepEqual(unpinned, []);
});

test('a release is dispatched by hand from main and reads the repository without write access', () => {
  const release = read('.github/workflows/release.yml');
  const triggers = release.split('\nconcurrency:')[0];
  assert.match(triggers, /^on:\n  workflow_dispatch:\n/m);
  assert.doesNotMatch(triggers, /schedule:|cron:|pull_request:|push:|workflow_run:/);
  const requireMain = release.indexOf('- name: Require main for releases');
  const selection = release.indexOf('run: node deploy/scripts/production-deployment-progress.mjs read');
  assert(requireMain >= 0 && selection > requireMain);
  assert.match(release.slice(requireMain, selection), /test "\$GITHUB_REF" = refs\/heads\/main/);
  assert.match(release.split('\njobs:')[0], /deployments: read/);
  assert.doesNotMatch(release.split('\njobs:')[0], /: write/);
});

test('CI runs on pull requests and Scope request branches', () => {
  const ci = read('.github/workflows/ci.yml');
  const release = read('.github/workflows/release.yml');
  const ciTriggers = ci.split('\nconcurrency:')[0];
  assert.match(ciTriggers, /  pull_request:\n  push:\n    branches: \['scope\/\*\*'\]\n/);
  assert.doesNotMatch(ciTriggers, /schedule:|workflow_dispatch:/);
  assert.match(ci, /group: scope-ci-\$\{\{ github\.event\.pull_request\.number \|\| github\.ref \}\}/);
  assert.match(ci, /base="\$\(git merge-base origin\/main HEAD\)"/);
  assert.match(ci, /BASE_SHA: \$\{\{ steps\.base\.outputs\.sha \}\}/);
  assert.doesNotMatch(ci, /github\.event\.before/);
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
  assert.match(gate, /RUN_ID: \$\{\{ needs\.pull-request\.outputs\.run_id \}\}/);
  assert.match(gate, /run: node \.github\/scripts\/report-pr-checks\.mjs/);
  assert.match(gate, /contents: read\n      pull-requests: read/);
  assert.ok(gate.indexOf('name: Checkout reporter') < gate.indexOf("name: Report the pull request's result"));
});

test('a Scope request revision reuses an open pull request run, waiting briefly for GitHub to start it', () => {
  const ci = read('.github/workflows/ci.yml');
  const script = stepScript(ci.slice(ci.indexOf('\n  pull-request:\n')), 'Find a pull request run that tests this commit');
  const find = ({ pulls = ['7'], runs = ['123'] }) => {
    const dir = mkdtempSync(resolve(tmpdir(), 'scope-ci-find-'));
    try {
      writeFileSync(resolve(dir, 'pulls'), pulls.join('\n') + '\n');
      writeFileSync(resolve(dir, 'runs'), runs.join('\n') + '\n');
      writeFileSync(resolve(dir, 'gh'), `#!/bin/bash\nanswer() { n=$(cat "${dir}/$1.count" 2>/dev/null || echo 0); echo $((n + 1)) > "${dir}/$1.count"; line=$((n + 1)); total=$(wc -l < "${dir}/$1"); reply=$(sed -n "$(( line < total ? line : total ))p" "${dir}/$1"); [[ "$reply" == error ]] && exit 1; echo "$reply"; }\ncase "$2" in\n  */pulls) answer pulls ;;\n  */runs*) answer runs ;;\n  *) exit 1 ;;\nesac\n`, { mode: 0o755 });
      writeFileSync(resolve(dir, 'sleep'), `#!/bin/sh\necho "$1" >> "${dir}/slept"\n`, { mode: 0o755 });
      const output = resolve(dir, 'output');
      const result = spawnSync('bash', ['-e', '-o', 'pipefail', '-c', script], {
        env: { ...process.env, PATH: `${dir}:${process.env.PATH}`, REPOSITORY: 'o/r', SHA: 'a'.repeat(40), GITHUB_OUTPUT: output },
        encoding: 'utf8',
      });
      assert.equal(result.status, 0, result.stderr);
      const slept = existsSync(resolve(dir, 'slept')) ? readFileSync(resolve(dir, 'slept'), 'utf8').trim().split('\n').length : 0;
      return { output: readFileSync(output, 'utf8'), slept };
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  };
  assert.deepEqual(find({}), { output: 'number=7\nrun_id=123\n', slept: 0 });
  assert.deepEqual(find({ pulls: ['', '7'], runs: ['', '123'] }), { output: 'number=7\nrun_id=123\n', slept: 2 }, 'the pull request and its run can appear after the push');
  assert.deepEqual(find({ pulls: [''] }), { output: 'number=\nrun_id=\n', slept: 7 });
  assert.deepEqual(find({ runs: [''] }), { output: 'number=\nrun_id=\n', slept: 7 }, 'a pull request with conflicts never gets a run to wait for');
  assert.deepEqual(find({ pulls: ['error', '7'] }), { output: 'number=7\nrun_id=123\n', slept: 1 }, 'a failed pull request lookup moves on to the next attempt');
  assert.deepEqual(find({ runs: ['error', '123'] }), { output: 'number=7\nrun_id=123\n', slept: 1 }, 'a failed run lookup moves on to the next attempt');
});

test('validation gate allows unselected jobs but fails selected jobs and cancellation', () => {
  const workflow = read('.github/workflows/validate.yml');
  const job = workflow.slice(workflow.indexOf('\n  production-validation-gate:\n'));
  assert.match(job, /if: \$\{\{ always\(\) \}\}\n/);
  assert.match(job, /if: \$\{\{ cancelled\(\) \}\}\n\s+run: exit 1\n/);
  const expression = job.match(/VALIDATIONS_PASSED: >-\n\s*\$\{\{ ([\s\S]*?) \}\}/)?.[1];
  assert.ok(expression, 'validation predicate must be evaluated in the assertion, not the job condition');
  const evaluate = Function('inputs', 'needs', `return (${expression.replace(/needs\.([\w-]+)/g, 'needs["$1"]')});`);
  const selectedBy = {
    'server-validation': ['backend', 'web'],
    'cli-validation': ['cli'],
    'integration-validation': ['web', 'cli', 'media_smoke'],
    'dependency-advisories': ['release'],
  };
  for (const mask of Array.from({ length: 32 }, (_, i) => i)) {
    const inputs = Object.fromEntries(['backend', 'web', 'cli', 'media_smoke'].map((key, index) => [key, String(Boolean(mask & (1 << index)))]));
    inputs.validate_server = true;
    inputs.release = Boolean(mask & 16);
    const needs = Object.fromEntries(Object.entries(selectedBy).map(([name, keys]) => [name, {
      result: keys.some((key) => String(inputs[key]) === 'true') ? 'success' : 'skipped',
    }]));
    needs['server-validation'].result = 'success';
    assert.equal(evaluate(inputs, needs), true, `selection ${mask}`);
    for (const [name, state] of Object.entries(needs)) {
      if (state.result !== 'success') continue;
      for (const result of ['failure', 'cancelled', 'skipped']) {
        assert.equal(evaluate(inputs, { ...needs, [name]: { result } }), false, `${mask}: ${name}/${result}`);
      }
    }
  }
  for (const value of ['true', 'false', '']) {
    const result = spawnSync('bash', ['-e', '-o', 'pipefail', '-c', validationGateScript(job)], {
      env: { ...process.env, VALIDATIONS_PASSED: value }, encoding: 'utf8',
    });
    assert.equal(result.status === 0, value === 'true', `assertion ${value}`);
  }
});

test('preparation overlaps validation while production waits for every check', () => {
  const release = read('.github/workflows/release.yml');
  const section = (name) => release.split(`\n  ${name}:\n`)[1].split(/\n  [\w-]+:\n/)[0];
  const needs = (name) => section(name).match(/needs: \[(.+)\]/)[1].split(', ');
  assert.deepEqual(needs('release-preparation'), ['plan', 'server-validation']);
  assert.deepEqual(needs('production-deploy'), ['plan', 'policy', 'ops', 'validation', 'release-preparation']);
  assert.deepEqual(needs('cli-deploy'), ['plan', 'policy', 'ops', 'validation', 'production-deploy']);
  assert.doesNotMatch(section('plan'), /dev\/checks\/(policy|ops)/);
  for (const job of ['policy', 'ops']) assert.doesNotMatch(section(job), /needs:/);
});

test('server readiness rejects each failed selected build before packaging', () => {
  const workflow = read('.github/workflows/validate-server.yml');
  const expression = workflow.match(/VALIDATIONS_PASSED: >-\n\s*\$\{\{ ([\s\S]*?) \}\}/)[1];
  const evaluate = Function('inputs', 'needs', `return (${expression.replace(/needs\.([\w-]+)/g, 'needs["$1"]')});`);
  for (const backend of ['true', 'false']) for (const web of ['true', 'false']) {
    const inputs = { backend, web };
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
