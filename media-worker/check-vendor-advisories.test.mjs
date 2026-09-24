import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { checkVendorAdvisories } from './check-vendor-advisories.mjs';

const vendor = JSON.parse(readFileSync(new URL('./codec-vendor.cdx.json', import.meta.url), 'utf8'));
const sbom = { bomFormat: 'CycloneDX', components: [...vendor.components] };
const advisory = (events) => ({
  id: 'OSV-test',
  affected: [{ ranges: [{ type: 'GIT', events }] }],
});

test('clean vendor response passes', () => {
  assert.deepEqual(checkVendorAdvisories(sbom, {}, vendor), { vulnerabilities: [], fixable: [] });
});

test('affected advisory with no published fix is retained', () => {
  const response = { vulns: [advisory([{ introduced: 'abc' }])] };
  const result = checkVendorAdvisories(sbom, response, vendor);
  assert.equal(result.vulnerabilities.length, 1);
  assert.equal(result.fixable.length, 0);
});

test('affected advisory with a published fix fails the gate', () => {
  const response = { vulns: [advisory([{ introduced: 'abc' }, { fixed: 'def' }])] };
  const dir = mkdtempSync(join(tmpdir(), 'scope-vendor-advisories-'));
  try {
    const sbomPath = join(dir, 'sbom.json');
    const responsePath = join(dir, 'response.json');
    writeFileSync(sbomPath, JSON.stringify(sbom));
    writeFileSync(responsePath, JSON.stringify(response));
    const result = spawnSync(process.execPath, [
      new URL('./check-vendor-advisories.mjs', import.meta.url).pathname,
      sbomPath,
      responsePath,
    ], { encoding: 'utf8' });
    assert.equal(result.status, 1);
    assert.match(result.stdout, /1 with a published fix/);
  } finally {
    rmSync(dir, { recursive: true });
  }
});

test('malformed and paginated responses fail closed', () => {
  assert.throws(() => checkVendorAdvisories(sbom, { vulns: {} }, vendor));
  assert.throws(() => checkVendorAdvisories(sbom, { next_page_token: 'more' }, vendor));
  assert.throws(() => checkVendorAdvisories({ ...sbom, components: [] }, {}, vendor));
});

test('unavailable response file fails the executable check', () => {
  const result = spawnSync(process.execPath, [
    new URL('./check-vendor-advisories.mjs', import.meta.url).pathname,
    new URL('./codec-vendor.cdx.json', import.meta.url).pathname,
    '/missing-osv-response.json',
  ], { encoding: 'utf8' });
  assert.equal(result.status, 1);
  assert.match(result.stderr, /ENOENT/);
});
