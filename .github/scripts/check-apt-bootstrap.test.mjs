import assert from 'node:assert/strict';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { resolve } from 'node:path';
import test from 'node:test';

const root = resolve(import.meta.dirname, '../..');
const read = path => readFileSync(resolve(root, path), 'utf8');
const gitInstaller = read('deploy/railway/install-git.sh');
const dockerfiles = execFileSync('git', ['ls-files'], { cwd: root, encoding: 'utf8' }).trim().split('\n').filter(path => /(?:^|\/)(?:Dockerfile|[^/]+\.Dockerfile)$/.test(path));

function verifyBootstrap(layer, version, distro, label) {
  const bounded = layer.match(/timeout --verbose --kill-after=(\d+)s (\d+)s sh -ec '([^']+)'/);
  assert.ok(bounded, `${label}: first APT operation must bootstrap under a deadline`);
  assert.ok(Number(bounded[1]) > 0 && Number(bounded[2]) > 0 && Number(bounded[1]) + Number(bounded[2]) < 360, label);
  const operations = [...bounded[3].matchAll(/apt-get ([^&]+)/g)].map(match => match[1].trim().replaceAll(/\s+/g, ' '));
  assert.equal(layer.match(/\bapt-get\b/g).length, 2, label);
  assert.deepEqual(operations, [
    '-o Acquire::Retries=0 -o Acquire::http::Timeout=30 update',
    `-o Acquire::Retries=0 -o Acquire::http::Timeout=30 install --yes --no-install-recommends "apt=\${APT_VERSION}" "${distro.library}=\${APT_VERSION}"`,
  ], label);
  assert.equal(version, distro.version, `${label}: pin the vendor-fixed APT release`);
  assert.match(layer, new RegExp(`'URIs: ${distro.uri}' 'Suites: ${distro.suite}-security'`), label);
  if (label === 'runner-runtime/Dockerfile:final') {
    assert.match(layer, /mv \/etc\/apt\/sources\.list\.d\/debian\.sources \/tmp\/debian\.sources/, label);
    assert.match(layer, /mv \/tmp\/debian\.sources \/etc\/apt\/sources\.list\.d\/debian\.sources/, label);
  } else {
    assert.match(layer, /mv \/etc\/apt\/sources\.list\.d \/tmp\/apt-sources\.list\.d/, label);
    assert.match(layer, /mv \/etc\/apt\/sources\.list \/tmp\/apt-sources\.list/, label);
    assert.match(layer, /mv \/tmp\/apt-sources\.list\.d \/etc\/apt\/sources\.list\.d/, label);
    assert.match(layer, /mv \/tmp\/apt-sources\.list \/etc\/apt\/sources\.list/, label);
    assert.ok(layer.indexOf('rm -rf /var/lib/apt/lists/*') < layer.indexOf('apt-get'), `${label}: discard inherited package indexes`);
  }
}

test('image stages bootstrap fixed APT from security with retries disabled before other package downloads', () => {
  const distros = {
    bookworm: { version: '2.6.2', suite: 'bookworm', uri: 'http://deb.debian.org/debian-security', library: 'libapt-pkg6.0' },
    trixie: { version: '3.0.4', suite: 'trixie', uri: 'http://deb.debian.org/debian-security', library: 'libapt-pkg7.0' },
    noble: { version: '2.8.4ubuntu0.1', suite: 'noble', uri: 'http://security.ubuntu.com/ubuntu', library: 'libapt-pkg6.0t64' },
  };
  const failures = [];
  for (const path of dockerfiles) {
    const aliases = new Map();
    const text = read(path).replaceAll(/\\\n\s*/g, ' ');
    const globalVersion = text.split(/^FROM /m)[0].match(/^ARG APT_VERSION=(\S+)$/m)?.[1];
    let stage;
    for (const line of text.split('\n')) {
      if (line.startsWith('FROM ')) {
        const base = line.replace(/^FROM (?:--\S+ )?/, '').split(' ')[0];
        const name = line.match(/ AS (\S+)$/i)?.[1] ?? 'final';
        stage = { name, distro: aliases.get(base)?.distro ?? (/ubuntu:24\.04/.test(base) ? 'noble' : /postgres:18|trixie/.test(base) ? 'trixie' : /bookworm/.test(base) ? 'bookworm' : undefined), fixed: aliases.get(base)?.fixed ?? false };
        aliases.set(name, stage);
      }
      if (!stage) continue;
      if (line.startsWith('ARG APT_VERSION')) stage.version = line.split('=')[1] ?? globalVersion;
      if (!line.startsWith('RUN ')) continue;
      const script = line.includes('/tmp/install-git.sh') ? gitInstaller.replaceAll(/\\\n\s*/g, ' ').split('\napt-get update')[0] : line;
      if (!/\bapt-get\b/.test(script) || stage.fixed) continue;
      try {
        const distro = distros[stage.distro];
        assert.ok(distro, `${path}:${stage.name}: unknown APT distribution`);
        verifyBootstrap(script, stage.version ?? script.match(/^export APT_VERSION=(\S+)$/m)?.[1], distro, `${path}:${stage.name}`);
        stage.fixed = true;
      } catch (error) {
        failures.push(error.message);
      }
    }
  }
  assert.deepEqual(failures, []);
});

test('bootstrap sources exclude inherited archives and cached indexes, then restore the original sources', () => {
  const layers = dockerfiles.flatMap(path => read(path).replaceAll(/\\\n\s*/g, ' ').split('\n').filter(line => line.startsWith('RUN ') && line.includes('apt-security.sources')).map(line => ({ path, script: line.slice(4) })));
  layers.push({ path: 'deploy/railway/install-git.sh', script: gitInstaller.split('\napt-get update')[0] });
  for (const { path, script } of layers) {
    const directory = mkdtempSync(resolve(tmpdir(), 'scope-apt-sources-'));
    try {
      const apt = resolve(directory, 'etc/apt');
      const lists = resolve(directory, 'var/lib/apt/lists');
      const sources = path === 'runner-runtime/Dockerfile'
        ? [['sources.list.d/debian.sources', 'inherited Debian source']]
        : [['sources.list', 'inherited legacy source'], ['sources.list.d/pgdg.list', 'inherited PostgreSQL source']];
      mkdirSync(`${apt}/sources.list.d`, { recursive: true });
      mkdirSync(lists, { recursive: true });
      mkdirSync(`${directory}/bin`);
      for (const [name, content] of sources) writeFileSync(`${apt}/${name}`, content);
      if (path !== 'runner-runtime/Dockerfile') writeFileSync(`${lists}/inherited-Packages`, 'inherited package index');
      writeFileSync(`${directory}/bin/apt-get`, '#!/bin/sh\nset -eu\ntest ! -f "$APT_ROOT/sources.list"\ntest "$(ls "$APT_ROOT/sources.list.d")" = apt-security.sources\ntest -z "$(ls "$APT_LISTS")"\ncat "$APT_ROOT/sources.list.d/apt-security.sources"\n', { mode: 0o755 });
      const isolated = script.replaceAll('/etc/apt', apt).replaceAll('/var/lib/apt/lists', lists).replaceAll('/tmp/apt-sources', `${directory}/saved-apt-sources`).replaceAll('/tmp/debian.sources', `${directory}/debian.sources`);
      const result = spawnSync('bash', ['-ec', isolated, 'bootstrap-probe', '2.55.0', '0'.repeat(64)], { env: { ...process.env, PATH: `${directory}/bin:${process.env.PATH}`, APT_VERSION: 'test-pin', IMAGE_DEPENDENCY_EPOCH: 'fixture', APT_ROOT: apt, APT_LISTS: lists }, encoding: 'utf8' });
      assert.equal(result.status, 0, `${path}: ${result.stderr}`);
      const observed = result.stdout.trim().split('\n');
      assert.equal(observed.length, 10, path);
      assert.equal(observed.filter(line => /^Suites: (bookworm|trixie|noble)-security$/.test(line)).length, 2, path);
      assert.equal(observed.filter(line => /^URIs: http:\/\/(deb.debian.org\/debian-security|security.ubuntu.com\/ubuntu)$/.test(line)).length, 2, path);
      for (const [name, content] of sources) assert.equal(readFileSync(`${apt}/${name}`, 'utf8'), content, path);
    } finally {
      rmSync(directory, { recursive: true, force: true });
    }
  }
});

test('the shared Git bootstrap installs only the fixed pair and stops on an APT failure', () => {
  const script = gitInstaller.replaceAll(/\\\n\s*/g, ' ');
  const bounded = script.match(/timeout --verbose --kill-after=10s 300s sh -ec '([^']+)'/);
  assert.ok(bounded);
  const directory = mkdtempSync(resolve(tmpdir(), 'scope-apt-bootstrap-'));
  try {
    mkdirSync(resolve(directory, 'bin'));
    writeFileSync(resolve(directory, 'bin/apt-get'), '#!/bin/sh\nprintf "%s\\n" "$*"\nexit "${APT_EXIT_STATUS:-0}"\n', { mode: 0o755 });
    const env = { ...process.env, PATH: `${directory}/bin:${process.env.PATH}`, APT_VERSION: '2.6.2' };
    const success = spawnSync('sh', ['-ec', bounded[1]], { env, encoding: 'utf8' });
    assert.equal(success.status, 0, success.stderr);
    assert.deepEqual(success.stdout.trim().split('\n'), [
      '-o Acquire::Retries=0 -o Acquire::http::Timeout=30 update',
      '-o Acquire::Retries=0 -o Acquire::http::Timeout=30 install --yes --no-install-recommends apt=2.6.2 libapt-pkg6.0=2.6.2',
    ]);
    const failure = spawnSync('sh', ['-ec', bounded[1]], { env: { ...env, APT_EXIT_STATUS: '100' }, encoding: 'utf8' });
    assert.equal(failure.status, 100, failure.stderr);
    assert.equal(failure.stdout.trim().split('\n').length, 1);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
