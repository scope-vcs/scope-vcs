import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import test from 'node:test';
import { cliReleaseManifest } from './cli-release-manifest.mjs';

test('a CLI release pins every distribution artifact to its checksum under a revision tag', () => {
  const sourceSha = '0123456789abcdef0123456789abcdef01234567';
  const targets = [{ artifact: 'scope-linux.tar.gz' }, { artifact: 'scope-windows.tar.gz' }];
  const manifest = cliReleaseManifest(sourceSha, targets, (artifact) => Buffer.from(artifact));

  assert.deepEqual(manifest, {
    tag: 'cli-0123456789ab',
    sourceSha,
    artifacts: Object.fromEntries(targets.map(({ artifact }) => [
      artifact, createHash('sha256').update(artifact).digest('hex'),
    ])),
  });
  assert.throws(() => cliReleaseManifest(sourceSha, targets, () => Buffer.alloc(0)), /scope-linux\.tar\.gz is empty/);
  assert.throws(() => cliReleaseManifest('0123456', targets, () => Buffer.from('x')), /full source revision/);
});
