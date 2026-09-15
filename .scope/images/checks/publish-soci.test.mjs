import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, copyFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import test from 'node:test';

const executable = `sha256:${'a'.repeat(64)}`;
const artifact = `sha256:${'b'.repeat(64)}`;
const source = `sha256:${'c'.repeat(64)}`;
const manifest = () => ({
  mediaType: 'application/vnd.oci.image.index.v1+json',
  manifests: [
    { digest: executable, mediaType: 'application/vnd.oci.image.manifest.v1+json',
      platform: { os: 'linux', architecture: 'amd64' }, annotations: { 'com.amazon.soci.index-digest': artifact } },
    { digest: artifact, annotations: { 'com.amazon.soci.image-manifest-digest': executable } },
  ],
});

function publish(t, index, scanStatus = 0, sourceImage = `ghcr.io/scope/checks@${source}`) {
  const root = mkdtempSync(join(tmpdir(), 'scope-soci-scan-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const path of ['bin', '.github/scripts', '.scope/images/checks']) mkdirSync(join(root, path), { recursive: true });
  copyFileSync(new URL('../../../.github/scripts/publish-soci-image.sh', import.meta.url), join(root, '.github/scripts/publish-soci-image.sh'));
  const script = (path, body) => writeFileSync(join(root, path), `#!/bin/bash\nset -euo pipefail\n${body}\n`, { mode: 0o755 });
  writeFileSync(join(root, 'manifest.json'), JSON.stringify(index));
  script('bin/aws', `case "$2" in\n batch-get-image) cat "$FIXTURE_ROOT/manifest.json" ;;\n describe-images) echo application/vnd.amazon.soci.index.v2+json ;;\n *) exit 90 ;;\nesac`);
  for (const name of ['skopeo', 'soci']) script(`bin/${name}`, 'printf "%s\\n" "$*" >> "$FIXTURE_ROOT/copy-commands"');
  script('.scope/images/checks/scan-image.sh', 'printf "%s\\n" "$@" > "$FIXTURE_ROOT/scan-args"\nexit "$SCAN_STATUS"');
  const result = spawnSync('bash', [join(root, '.github/scripts/publish-soci-image.sh'), sourceImage,
    'registry.test/scope/checks:raw', 'registry.test/scope/checks:soci', 'scope/checks', 'raw', 'soci'], {
    cwd: root, encoding: 'utf8', env: { ...process.env, PATH: `${root}/bin:${process.env.PATH}`, FIXTURE_ROOT: root, SCAN_STATUS: String(scanStatus) },
  });
  return { root, result };
}

test('scans the immutable executable child after copying the verified source digest', (t) => {
  const { root, result } = publish(t, manifest());
  assert.equal(result.status, 0, result.stderr);
  assert.equal(readFileSync(join(root, 'scan-args'), 'utf8'), `remote\nregistry.test/scope/checks@${executable}\nchecks-image-soci-scan.json\n`);
  assert.match(readFileSync(join(root, 'copy-commands'), 'utf8'), new RegExp(`docker://ghcr.io/scope/checks@${source}`));
});

test('propagates a failed or unavailable scan to prevent promotion', (t) => {
  for (const status of [1, 2, 124]) assert.equal(publish(t, manifest(), status).result.status, status);
});

test('rejects mutable source tags and malformed or ambiguous executable children', (t) => {
  assert.notEqual(publish(t, manifest(), 0, 'ghcr.io/scope/checks:main').result.status, 0);
  const wrongPlatform = manifest(); wrongPlatform.manifests[0].platform.architecture = 'arm64';
  const badDigest = manifest(); badDigest.manifests[0].digest = 'invalid';
  const extraChild = manifest(); extraChild.manifests.push({ ...extraChild.manifests[0], digest: source });
  for (const index of [{}, wrongPlatform, badDigest, extraChild]) assert.notEqual(publish(t, index).result.status, 0);
});

test('workflow gates local publishing and mutable main/release promotion on scans', () => {
  const workflow = readFileSync(new URL('../../../.github/workflows/scope-checks-image.yml', import.meta.url), 'utf8');
  assert(workflow.indexOf('- name: Scan verified executable image') < workflow.indexOf('- name: Publish verified image'));
  const childScan = workflow.indexOf('- name: Publish raw and SOCI v2 variants');
  const promotion = workflow.indexOf('- name: Promote scanned image to main');
  assert(childScan > 0 && promotion > childScan);
  assert(workflow.indexOf('- name: Record published digest') > promotion);
  assert.equal((workflow.match(/docker push "\$\{GHCR_IMAGE_NAME\}:main"/g) ?? []).length, 1);
  assert.match(workflow, /GHCR_IMAGE: \$\{\{ env.GHCR_IMAGE_NAME \}\}@\$\{\{ steps.image.outputs.digest \}\}/);
});
