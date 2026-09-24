import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import test from 'node:test';
import { smokeToolsEvidence, verifySmokeTools } from './smoke-tools-evidence.mjs';

test('smoke tools are bound to source, toolchain and exact archive bytes', (t) => {
  const dir = mkdtempSync(join(tmpdir(), 'smoke-tools-'));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const sha = 'a'.repeat(40);
  const sourceToolchain = join(dir, 'source-rust-toolchain.toml');
  writeFileSync(sourceToolchain, '[toolchain]\nchannel = "1.97.0"\n');
  writeFileSync(join(dir, 'staging-commands.tar.gz'), 'archive bytes');
  const evidence = smokeToolsEvidence(dir, sha, sourceToolchain);
  const save = (value) => writeFileSync(join(dir, 'staging-commands.json'), JSON.stringify(value));
  save(evidence);
  const currentToolchain = join(dir, 'current-rust-toolchain.toml');
  writeFileSync(currentToolchain, '[toolchain]\nchannel = "1.98.1"\n');
  assert.throws(() => verifySmokeTools(dir, sha, currentToolchain), /rustToolchain mismatch/);
  assert.deepEqual(verifySmokeTools(dir, sha, sourceToolchain), evidence);
  assert.throws(() => verifySmokeTools(dir, 'b'.repeat(40), sourceToolchain), /sourceSha mismatch/);
  save({ ...evidence, rustToolchain: 'unreviewed' });
  assert.throws(() => verifySmokeTools(dir, sha, sourceToolchain), /rustToolchain mismatch/);
  save(evidence);
  writeFileSync(join(dir, 'staging-commands.tar.gz'), 'changed bytes');
  assert.throws(() => verifySmokeTools(dir, sha, sourceToolchain), /archiveSha256 mismatch/);
  assert.throws(() => smokeToolsEvidence(dir, 'main', sourceToolchain), /exact source SHA/);
});
