import assert from 'node:assert/strict';
import test from 'node:test';
import { actionableFindings } from './scan-report.mjs';

const finding = (fields = {}) => ({
  VulnerabilityID: 'CVE-2026-0001', PkgName: 'libssl3', Severity: 'HIGH',
  InstalledVersion: '1', FixedVersion: '2', ...fields,
});
const report = (findings = []) => ({
  SchemaVersion: 2, Metadata: { OS: { Family: 'debian' } },
  Results: [{ Class: 'os-pkgs', Type: 'debian', Target: 'Debian', Vulnerabilities: findings }],
});

test('blocks fixed high and critical findings across OS and language packages', () => {
  const input = report([finding(), finding({ Severity: 'MEDIUM' })]);
  input.Results.push({ Class: 'lang-pkgs', Target: 'package-lock.json', Vulnerabilities: [finding({ Severity: 'CRITICAL', PkgName: 'npm-package' })] });
  assert.deepEqual(actionableFindings(input).map((item) => item.PkgName), ['libssl3', 'npm-package']);
});

test('allows a complete clean report and retains unfixed findings without blocking', () => {
  assert.deepEqual(actionableFindings(report()), []);
  assert.deepEqual(actionableFindings(report([finding({ FixedVersion: '' }), finding({ FixedVersion: undefined })])), []);
});

test('does not suppress fixable kernel-header findings', () => {
  assert.equal(actionableFindings(report([finding({ PkgName: 'linux-libc-dev' })])).length, 1);
});

test('rejects absent, unsupported and malformed scan output', () => {
  for (const input of [null, {}, { ...report(), SchemaVersion: 1 }, { ...report(), Results: [] },
    { ...report(), Metadata: { OS: { Family: 'unknown' } } },
    report([finding({ Severity: 'INVALID' })]), report([finding({ FixedVersion: false })]),
    report([{}]), report(false)]) {
    assert.throws(() => actionableFindings(input));
  }
});
