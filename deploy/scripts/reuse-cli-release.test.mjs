import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import test from 'node:test';

const script = resolve('deploy/scripts/reuse-cli-release.sh');
const sourceSha = '0123456789abcdef0123456789abcdef01234567';
const tag = 'cli-0123456789ab';
const targets = { targets: [{ artifact: 'scope-linux.tar.gz' }, { artifact: 'scope-windows.tar.gz' }] };
const archives = { 'scope-linux.tar.gz': 'linux build', 'scope-windows.tar.gz': 'windows build' };
const published = {
  tag, sourceSha,
  artifacts: Object.fromEntries(Object.entries(archives)
    .map(([artifact, bytes]) => [artifact, createHash('sha256').update(bytes).digest('hex')])),
};

function reuse(t, release) {
  const directory = mkdtempSync(join(tmpdir(), 'scope-cli-rerun-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  mkdirSync(join(directory, 'cli/distribution'), { recursive: true });
  mkdirSync(join(directory, 'bin'));
  writeFileSync(join(directory, 'cli/distribution/targets.json'), JSON.stringify(targets));
  if (release) writeFileSync(join(directory, 'release.json'), JSON.stringify(release));
  writeFileSync(join(directory, 'bin/gh'), `#!/usr/bin/env bash
set -euo pipefail
printf '%s\\n' "$1 $2" >> "$FAKE_GH_CALLS"
case "$1 $2" in
  "release view") [[ -f release.json ]] || exit 1; jq '{isDraft: (.draft // false), assets: [.assets[] | {name: .}]}' release.json ;;
  "release download")
    jq '.manifest' release.json > "\${@: -1}/cli-release.json"
    jq -r '.archives | to_entries[] | "\\(.key)\t\\(.value)"' release.json | while IFS=$'\t' read -r name bytes; do
      printf '%s' "$bytes" > "\${@: -1}/$name"
    done ;;
  "attestation verify") [[ "$(jq -r '.attested' release.json)" == true ]] ;;
  *) exit 2 ;;
esac
`, { mode: 0o755 });
  const result = spawnSync('bash', [script, tag, 'cli/dist/cli-release.json'], {
    cwd: directory, encoding: 'utf8',
    env: { ...process.env, PATH: `${join(directory, 'bin')}:${process.env.PATH}`,
      GITHUB_REPOSITORY: 'scope-vcs/scope-vcs', GITHUB_SHA: sourceSha, FAKE_GH_CALLS: join(directory, 'calls') },
  });
  const manifest = join(directory, 'cli/dist/cli-release.json');
  return {
    ...result,
    calls: readFileSync(join(directory, 'calls'), 'utf8').trim().split('\n'),
    manifest: existsSync(manifest) ? JSON.parse(readFileSync(manifest, 'utf8')) : null,
  };
}

const assets = ['scope-linux.tar.gz', 'scope-windows.tar.gz', 'cli-release.json'];

test('a rerun at a published revision reuses the release manifest instead of comparing rebuilt archives', (t) => {
  const fresh = reuse(t, null);
  assert.equal(fresh.status, 0, fresh.stderr);
  assert.equal(fresh.stdout, 'exists=false\n');
  assert.equal(fresh.manifest, null);

  const rerun = reuse(t, { assets, archives, manifest: published, attested: true });
  assert.equal(rerun.status, 0, rerun.stderr);
  assert.equal(rerun.stdout, 'exists=true\n');
  assert.deepEqual(rerun.manifest, published);
  assert.deepEqual(rerun.calls, ['release view', 'release download', 'attestation verify']);
});

test('a draft, incomplete, unattested, foreign or altered release is not reused', (t) => {
  const usable = { assets, archives, manifest: published, attested: true };
  for (const [change, expected] of [
    [{ draft: true }, /is a draft that nobody can download/],
    [{ assets: assets.slice(1) }, /lacks scope-linux\.tar\.gz/],
    [{ attested: false }, /^$/],
    [{ manifest: { ...published, sourceSha: 'f'.repeat(40) } }, /does not describe this revision/],
    [{ manifest: { ...published, artifacts: { 'scope-linux.tar.gz': published.artifacts['scope-linux.tar.gz'] } } }, /does not describe this revision/],
    [{ archives: { ...archives, 'scope-windows.tar.gz': 'replaced build' } }, /serves scope-windows\.tar\.gz with a digest its attested manifest does not record/],
  ]) {
    const result = reuse(t, { ...usable, ...change });
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, expected);
    assert.equal(result.manifest, null);
    assert.doesNotMatch(result.stdout, /exists=true/);
  }
});
