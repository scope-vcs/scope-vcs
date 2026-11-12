import assert from 'node:assert/strict';
import { randomBytes } from 'node:crypto';
import { mkdtemp, readFile, readdir, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { snapshotKey, transformSnapshot } from './staging-baseline-crypto.mjs';

async function fixture(t) {
  const directory = await mkdtemp(join(tmpdir(), 'scope-baseline-crypto-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const input = join(directory, 'database.dump');
  const encrypted = join(directory, 'database.dump.enc');
  const output = join(directory, 'restored.dump');
  const metadata = join(directory, 'baseline.json');
  const bytes = randomBytes(1024 * 1024 + 17);
  const key = randomBytes(32).toString('hex');
  await writeFile(input, bytes);
  await writeFile(metadata, JSON.stringify({ environmentId: 'staging', ledgerHash: 'baseline', metadataRestoreSafe: true }));
  return { directory, input, encrypted, output, metadata, bytes, key };
}

test('encrypted snapshots round-trip with independent nonces and private plaintext files', async (t) => {
  const f = await fixture(t);
  await transformSnapshot('encrypt', f.input, f.encrypted, f.metadata, f.key);
  const first = await readFile(f.encrypted);
  assert.notDeepEqual(first, f.bytes);
  await transformSnapshot('encrypt', f.input, f.encrypted, f.metadata, f.key);
  assert.notDeepEqual(await readFile(f.encrypted), first);
  await transformSnapshot('decrypt', f.encrypted, f.output, f.metadata, f.key);
  assert.deepEqual(await readFile(f.output), f.bytes);
  assert.equal((await stat(f.output)).mode & 0o777, 0o600);
  assert.equal((await stat(f.encrypted)).mode & 0o777, 0o600);
});

for (const change of ['version', 'nonce', 'ciphertext', 'tag', 'truncated', 'key', 'metadata', 'plaintext']) {
  test(`${change} mismatch never publishes unauthenticated plaintext`, async (t) => {
    const f = await fixture(t);
    await transformSnapshot('encrypt', f.input, f.encrypted, f.metadata, f.key);
    let encrypted = await readFile(f.encrypted);
    if (change === 'version') encrypted[0] ^= 1;
    if (change === 'nonce') encrypted[25] ^= 1;
    if (change === 'ciphertext') encrypted[100] ^= 1;
    if (change === 'tag') encrypted[encrypted.length - 1] ^= 1;
    if (change === 'truncated') encrypted = encrypted.subarray(0, 10);
    if (change === 'plaintext') encrypted = f.bytes;
    if (change === 'metadata') await writeFile(f.metadata, '{"environmentId":"production","metadataRestoreSafe":true}');
    await writeFile(f.encrypted, encrypted);
    await writeFile(f.output, 'previous verified output');
    await assert.rejects(transformSnapshot('decrypt', f.encrypted, f.output, f.metadata,
      change === 'key' ? randomBytes(32).toString('hex') : f.key));
    assert.equal(await readFile(f.output, 'utf8'), 'previous verified output');
    assert.equal((await readdir(f.directory)).some(name => name.endsWith('.tmp')), false);
  });
}

test('missing, malformed, or differently sized keys are rejected without echoing their value', () => {
  for (const key of [undefined, '', 'f'.repeat(63), 'g'.repeat(64), 'secret-value', 'a'.repeat(66)]) {
    assert.throws(() => snapshotKey(key), { message: 'SCOPE_STAGING_BASELINE_KEY must contain exactly 64 hexadecimal characters.' });
  }
  assert.equal(snapshotKey('A'.repeat(64)).length, 32);
});
