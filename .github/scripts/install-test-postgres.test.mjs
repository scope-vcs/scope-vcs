import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import test from 'node:test';

const root = resolve(import.meta.dirname, '../..');
const read = path => readFileSync(resolve(root, path), 'utf8');
const action = read('.github/actions/install-test-postgres/action.yml');

function installer(t) {
  const directory = mkdtempSync(resolve(tmpdir(), 'scope-postgres-install-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  for (const path of ['bin', 'cache', 'etc', 'usr/lib/postgresql/18/bin']) {
    mkdirSync(resolve(directory, path), { recursive: true });
  }
  writeFileSync(resolve(directory, 'etc/os-release'), 'VERSION_CODENAME=noble\n');
  copyFileSync(resolve(root, 'dev/install-test-postgres.asc'), resolve(directory, 'install-test-postgres.asc'));
  writeFileSync(resolve(directory, 'install.sh'), read('dev/install-test-postgres.sh')
    .replaceAll('/etc/', `${directory}/etc/`).replaceAll('/usr/lib/postgresql/', `${directory}/usr/lib/postgresql/`));
  const probe = body => `#!${process.execPath}
const fs = require('node:fs');
const path = require('node:path');
const args = process.argv.slice(2);
const record = entry => fs.appendFileSync(process.env.OBSERVED, JSON.stringify({ tool: path.basename(process.argv[1]), ...entry }) + '\\n');
${body}
`;
  const apt = probe(`const options = {};
while (args[0] === '-o') {
  args.shift();
  const [key, ...value] = args.shift().split('=');
  options[key] = value.join('=');
}
const source = options['Dir::Etc::sourcelist'];
record({ args, options, sourceparts: fs.readdirSync(options['Dir::Etc::sourceparts']), sources: source ? fs.readFileSync(source, 'utf8') : 'inherited sources' });
if (args[0] === 'madison') console.log('postgresql-18 | 18.6-1.pgdg24.04+2 | fixture');
if (args.includes('--download-only')) {
  for (const [name, version] of [['postgresql-18', '18.6-1.pgdg24.04+2'], ['postgresql-common', '294.pgdg24.04+2'], ['liburing2', '2.5-1build1']]) {
    fs.writeFileSync(path.join(options['Dir::Cache::archives'], name + '.deb'), name + ' ' + version);
  }
}`);
  const dpkgDeb = probe(`if (args[0] === '--show') console.log(fs.readFileSync(args.at(-1), 'utf8'));
if (args[0] === '--extract') record({ package: path.basename(args[1]), target: args[2] });`);
  const installed = { 'postgresql-common': '294.pgdg24.04+2', liburing2: '2.4-1' };
  const dpkgQuery = probe(`const version = ${JSON.stringify(installed)}[args.at(-1)];
if (!version) process.exit(1);
process.stdout.write(version);`);
  const dpkg = probe(`process.exit(args[0] === '--compare-versions' && args[2] === 'ge' && args[1] === args[3] ? 0 : 1);`);
  for (const command of ['apt-get', 'apt-cache']) writeFileSync(resolve(directory, 'bin', command), apt, { mode: 0o755 });
  writeFileSync(resolve(directory, 'bin/dpkg-deb'), dpkgDeb, { mode: 0o755 });
  writeFileSync(resolve(directory, 'bin/dpkg-query'), dpkgQuery, { mode: 0o755 });
  writeFileSync(resolve(directory, 'bin/dpkg'), dpkg, { mode: 0o755 });
  for (const command of ['ldconfig', 'localedef']) writeFileSync(resolve(directory, 'bin', command), probe('record({ args });'), { mode: 0o755 });
  writeFileSync(resolve(directory, 'bin/locale'), probe("for (const name of (process.env.LOCALES ?? 'C C.utf8 POSIX').split(' ')) console.log(name);"), { mode: 0o755 });
  writeFileSync(resolve(directory, 'usr/lib/postgresql/18/bin/initdb'), '#!/bin/sh\necho "initdb (PostgreSQL) 18.6 (fixture)"\n', { mode: 0o755 });
  const observed = resolve(directory, 'observed');
  return {
    directory,
    operations: () => readFileSync(observed, 'utf8').trim().split('\n').map(line => JSON.parse(line)),
    run: (mode, env = {}) => spawnSync('bash', [resolve(directory, 'install.sh'), mode, resolve(directory, 'cache')], {
      env: { ...process.env, PATH: `${directory}/bin:${process.env.PATH}`, OBSERVED: observed, ...env }, encoding: 'utf8',
    }),
  };
}

test('cold downloads use signed isolated sources and warm installation unpacks only missing verified packages without apt', t => {
  const fixture = installer(t);
  const downloaded = fixture.run('download');
  assert.equal(downloaded.status, 0, downloaded.stderr);
  const cold = fixture.operations();
  assert.deepEqual(cold.map(operation => operation.args[0]), ['update', 'madison', 'install']);
  for (const operation of cold) {
    assert.deepEqual(operation.sourceparts, []);
    assert.doesNotMatch(operation.sources, /mirror\+|azure\.|trusted=yes|allow-insecure/);
    assert.ok(operation.sources.trim().split('\n').every(entry => /^deb \[signed-by=\S+\] https:\/\//.test(entry)));
    assert.match(operation.sources, /https:\/\/apt\.postgresql\.org\/pub\/repos\/apt noble-pgdg main 18/);
    assert.equal(operation.options['APT::Update::Error-Mode'], undefined);
  }
  for (const required of ['--download-only', 'postgresql-18=18.6-1.pgdg24.04+2', 'postgresql-client-18=18.6-1.pgdg24.04+2']) {
    assert.ok(cold[2].args.includes(required), required);
  }
  for (const excluded of ['--reinstall', 'locales', 'postgresql-common']) assert.ok(!cold[2].args.includes(excluded), excluded);
  const installed = fixture.run('install');
  assert.equal(installed.status, 0, installed.stderr);
  const warm = fixture.operations().slice(cold.length);
  assert.deepEqual(warm.filter(operation => /^apt/.test(operation.tool)), []);
  assert.deepEqual(warm.filter(operation => operation.tool === 'dpkg-deb').map(({ package: name, target }) => [name, target]).sort(), [
    ['liburing2.deb', '/'], ['postgresql-18.deb', '/'],
  ]);
  assert.ok(warm.some(operation => operation.tool === 'ldconfig'));
  assert.deepEqual(warm.filter(operation => operation.tool === 'localedef').map(operation => operation.args), [['-i', 'en_US', '-f', 'UTF-8', 'en_US.utf8']]);
  assert.match(installed.stdout, /\(PostgreSQL\) 18\.6/);
  writeFileSync(resolve(fixture.directory, 'cache/postgresql-18.deb'), 'corrupt-package');
  const corrupt = fixture.run('install');
  assert.notEqual(corrupt.status, 0);
  assert.match(corrupt.stderr, /checksum did NOT match/);
  assert.equal(fixture.operations().length, cold.length + warm.length);
});

test('installation keeps an en_US.utf8 locale the runner already has', t => {
  const fixture = installer(t);
  assert.equal(fixture.run('download').status, 0);
  const installed = fixture.run('install', { LOCALES: 'C C.utf8 en_US.utf8 POSIX' });
  assert.equal(installed.status, 0, installed.stderr);
  assert.ok(!fixture.operations().some(operation => operation.tool === 'localedef'));
});

test('a cached server package for another PostgreSQL version is refused before anything is unpacked', t => {
  const fixture = installer(t);
  assert.equal(fixture.run('download').status, 0);
  const before = fixture.operations().length;
  writeFileSync(resolve(fixture.directory, 'cache/postgresql-18.deb'), 'postgresql-18 18.7-1.pgdg24.04+1');
  spawnSync('bash', ['-c', 'sha256sum ./*.deb > SHA256SUMS'], { cwd: resolve(fixture.directory, 'cache') });
  const result = fixture.run('install');
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /Cached postgresql-18 18\.7-1\.pgdg24\.04\+1 is not PostgreSQL 18\.6/);
  assert.equal(fixture.operations().length, before);
});

test('a changed signing key stops downloads before apt runs', t => {
  const fixture = installer(t);
  writeFileSync(resolve(fixture.directory, 'install-test-postgres.asc'), 'wrong-key');
  const result = fixture.run('download');
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /checksum did NOT match/);
  assert.deepEqual(readdirSync(resolve(fixture.directory, 'cache')), []);
});

test('cache planning changes with the runner image, pinned PostgreSQL version, and installer content', t => {
  const directory = mkdtempSync(resolve(tmpdir(), 'scope-postgres-cache-plan-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  for (const path of ['dev', '.github/actions/install-test-postgres', 'temp']) mkdirSync(resolve(directory, path), { recursive: true });
  for (const path of ['dev/install-test-postgres.sh', 'dev/install-test-postgres.asc', '.github/actions/install-test-postgres/action.yml']) {
    copyFileSync(resolve(root, path), resolve(directory, path));
  }
  const plan = action.split('      run: |\n')[1].split('\n    - id: restore')[0];
  function key(imageVersion = '20261111.1') {
    const output = resolve(directory, 'output');
    writeFileSync(output, '');
    const result = spawnSync('bash', ['-ec', plan], { cwd: directory, encoding: 'utf8', env: {
      ...process.env, ImageOS: 'ubuntu24', ImageVersion: imageVersion, RUNNER_ARCH: 'X64', RUNNER_TEMP: resolve(directory, 'temp'), GITHUB_OUTPUT: output,
    } });
    assert.equal(result.status, 0, result.stderr);
    return readFileSync(output, 'utf8').match(/^key=(.+)$/m)[1];
  }
  const initial = key();
  assert.match(initial, /^test-postgres-ubuntu24-20261111\.1-X64-18\.6-/);
  assert.notEqual(key('20261112.1'), initial);
  const script = resolve(directory, 'dev/install-test-postgres.sh');
  writeFileSync(script, readFileSync(script, 'utf8').replace('postgres_version=18.6', 'postgres_version=18.7'));
  const upgraded = key();
  assert.match(upgraded, /-18\.7-/);
  assert.notEqual(upgraded, initial);
  writeFileSync(script, `${readFileSync(script, 'utf8')}\n`);
  assert.notEqual(key(), upgraded);
  const steps = action.split(/\n    - /).slice(1);
  const download = steps.find(step => step.includes('install-test-postgres.sh download'));
  const save = steps.find(step => step.includes('actions/cache/save@'));
  for (const step of [download, save]) assert.match(step, /^if: steps\.restore\.outputs\.cache-hit != 'true'/);
  assert.ok(action.includes('actions/cache/restore@'));
  assert.ok(action.indexOf('actions/cache/save@') < action.indexOf('install-test-postgres.sh install'));
  assert.doesNotMatch(action, /restore-keys:/);
});

test('all workflow installers use the shared action and main warms its exact cache after image or installer changes', () => {
  const workflows = readdirSync(resolve(root, '.github/workflows')).filter(path => /\.ya?ml$/.test(path));
  for (const path of workflows) {
    const source = read(`.github/workflows/${path}`);
    assert.doesNotMatch(source, /run:.*install-test-postgres\.sh/, path);
    for (const step of source.split(/\n      - /).filter(step => step.startsWith('name: Install PostgreSQL test server'))) {
      assert.match(step, /uses: \.\/\.github\/actions\/install-test-postgres/, path);
    }
  }
  const warm = read('.github/workflows/warm-ci-caches.yml');
  assert.match(warm, /branches: \[main\]/);
  assert.match(warm, /schedule:\n\s+- cron:/);
  assert.match(warm, /uses: \.\/\.github\/actions\/install-test-postgres/);
  for (const path of ['.github/actions/install-test-postgres/**', 'dev/install-test-postgres.sh', 'dev/install-test-postgres.asc']) {
    assert.ok(warm.includes(`- ${path}`), path);
  }
});
