import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import test from 'node:test';

const root = resolve(import.meta.dirname, '../..');

test('PostgreSQL installation uses direct signed sources and fresh indexes for every apt operation', t => {
  const directory = mkdtempSync(resolve(tmpdir(), 'scope-postgres-install-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  for (const path of ['bin', 'etc/apt/sources.list.d', 'etc/postgresql-common', 'usr/lib/postgresql/18/bin']) {
    mkdirSync(resolve(directory, path), { recursive: true });
  }
  writeFileSync(resolve(directory, 'etc/os-release'), 'VERSION_CODENAME=noble\n');
  const inherited = resolve(directory, 'etc/apt/sources.list.d/ubuntu.sources');
  const inheritedSources = 'Types: deb\nURIs: mirror+file:/etc/apt/apt-mirrors.txt\nSuites: noble\nComponents: main\n';
  writeFileSync(inherited, inheritedSources);
  writeFileSync(resolve(directory, 'etc/postgresql-common/createcluster.conf'), 'create_main_cluster = true\n');
  copyFileSync(resolve(root, 'dev/install-test-postgres.asc'), resolve(directory, 'install-test-postgres.asc'));
  const source = readFileSync(resolve(root, 'dev/install-test-postgres.sh'), 'utf8');
  writeFileSync(resolve(directory, 'install.sh'), source.replaceAll('/etc/', `${directory}/etc/`).replaceAll('/usr/lib/postgresql/', `${directory}/usr/lib/postgresql/`));
  const aptProbe = `#!${process.execPath}
const fs = require('node:fs');
const args = process.argv.slice(2);
const options = {};
while (args[0] === '-o') {
  args.shift();
  const [key, ...value] = args.shift().split('=');
  options[key] = value.join('=');
}
const source = options['Dir::Etc::sourcelist'];
const lists = options['Dir::State::lists'];
fs.appendFileSync(process.env.APT_OBSERVED, JSON.stringify({
  args, options, source, lists,
  sources: source ? fs.readFileSync(source, 'utf8') : 'inherited mirror+file sources',
  indexes: lists ? fs.readdirSync(lists).filter(name => name !== 'partial') : ['inherited index'],
}) + '\\n');
if (args[0] === 'madison') console.log('postgresql-18 | 18.6-1.pgdg24.04+2 | fixture');
`;
  for (const command of ['apt-get', 'apt-cache']) writeFileSync(resolve(directory, 'bin', command), aptProbe, { mode: 0o755 });
  writeFileSync(resolve(directory, 'bin/localedef'), '#!/bin/sh\nexit 0\n', { mode: 0o755 });
  writeFileSync(resolve(directory, 'usr/lib/postgresql/18/bin/initdb'), '#!/bin/sh\necho initdb-verified\n', { mode: 0o755 });
  const observed = resolve(directory, 'observed');
  const result = spawnSync('bash', [resolve(directory, 'install.sh')], {
    env: { ...process.env, PATH: `${directory}/bin:${process.env.PATH}`, APT_OBSERVED: observed }, encoding: 'utf8',
  });
  assert.equal(result.status, 0, result.stderr);
  const operations = readFileSync(observed, 'utf8').trim().split('\n').map(line => JSON.parse(line));
  assert.equal(operations.length, 4);
  for (const operation of operations) {
    assert.equal(operation.options['Dir::Etc::sourceparts'], '-');
    assert.deepEqual(operation.indexes, []);
    assert.doesNotMatch(operation.sources, /mirror\+|azure\.|trusted=yes|allow-insecure/);
    const entries = operation.sources.trim().split('\n');
    assert.ok(entries.every(entry => /^deb \[signed-by=\S+\] https:\/\//.test(entry)));
    assert.ok(entries.some(entry => /https:\/\/archive\.ubuntu\.com\/ubuntu noble main/.test(entry)));
    assert.ok(entries.some(entry => /https:\/\/security\.ubuntu\.com\/ubuntu noble-security main/.test(entry)));
    assert.ok(entries.some(entry => /https:\/\/apt\.postgresql\.org\/pub\/repos\/apt noble-pgdg main 18/.test(entry)));
    assert.equal(existsSync(operation.source), false);
  }
  assert.equal(operations[0].options['APT::Update::Error-Mode'], 'any');
  assert.ok(operations[3].args.includes('postgresql-18=18.6-1.pgdg24.04+2'));
  assert.match(result.stdout, /initdb-verified/);
  assert.equal(readFileSync(inherited, 'utf8'), inheritedSources);
});
