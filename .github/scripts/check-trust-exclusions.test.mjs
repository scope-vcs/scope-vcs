import assert from 'node:assert/strict'
import test from 'node:test'
import { expiredTrustExclusions, lapsedAdvisoryExceptions, trustExclusions } from './check-trust-exclusions.mjs'

const workspaceYaml = [
  'trustPolicy: no-downgrade',
  'trustPolicyExclude:',
  '  - semver@6.3.1',
  '  - "@pierre/theme@2.0.0"',
  'blockExoticSubdeps: true',
  'overrides:',
  '  - not-an-exclusion@1.0.0',
].join('\n')
const expiries = new Map([['@pierre/theme', '2027-01-28']])

test('reads the excluded packages and nothing after the list', () => {
  assert.deepEqual(trustExclusions(workspaceYaml), ['semver@6.3.1', '@pierre/theme@2.0.0'])
  assert.deepEqual(trustExclusions('trustPolicy: no-downgrade\n'), [])
})

test('an exclusion fails from its expiry date and passes before it', () => {
  assert.deepEqual(expiredTrustExclusions(workspaceYaml, '2027-01-27', expiries), [])
  assert.deepEqual(expiredTrustExclusions(workspaceYaml, '2027-01-28', expiries),
    [{ exclusion: '@pierre/theme@2.0.0', expiry: '2027-01-28' }])
})

test('a new version of an expiring package keeps the same expiry', () => {
  const bumped = workspaceYaml.replace('@pierre/theme@2.0.0', '@pierre/theme@2.0.1')
  assert.deepEqual(expiredTrustExclusions(bumped, '2027-02-01', expiries),
    [{ exclusion: '@pierre/theme@2.0.1', expiry: '2027-01-28' }])
})

test('blank lines inside the list do not hide later entries', () => {
  const spaced = workspaceYaml.replace('  - semver@6.3.1\n', '  - semver@6.3.1\n\n')
  assert.deepEqual(trustExclusions(spaced), ['semver@6.3.1', '@pierre/theme@2.0.0'])
  assert.deepEqual(expiredTrustExclusions(spaced, '2027-01-28', expiries),
    [{ exclusion: '@pierre/theme@2.0.0', expiry: '2027-01-28' }])
})

test('an exclusion list the check cannot read fails instead of passing', () => {
  assert.throws(() => trustExclusions('trustPolicyExclude: ["@pierre/theme@2.0.0"]\n'), /block list/)
  assert.throws(() => trustExclusions('trustPolicyExclude:\n  - semver@6.3.1\n  - name: "@pierre/theme@2.0.0"\n'),
    /cannot read/)
})

const auditYaml = [
  'blockExoticSubdeps: true',
  'auditConfig:',
  '  ignoreGhsas:',
  '    - GHSA-fc5q-39xc-vjpf',
  'supportedArchitectures:',
  '  os:',
  '    - linux',
].join('\n')
const advisoryExceptions = new Map([
  ['GHSA-fc5q-39xc-vjpf', { expiry: '2026-12-09', approver: 'Adam Blumoff', reason: 'build tooling only' }],
])

test('an ignored advisory fails from its expiry date and passes before it', () => {
  assert.deepEqual(lapsedAdvisoryExceptions(auditYaml, '2026-12-08', advisoryExceptions), [])
  assert.deepEqual(lapsedAdvisoryExceptions(auditYaml, '2026-12-09', advisoryExceptions),
    ['GHSA-fc5q-39xc-vjpf (build tooling only; approved by Adam Blumoff) expired on 2026-12-09'])
})

test('an ignored advisory without a recorded approval fails', () => {
  const unrecorded = auditYaml.replace('    - GHSA-fc5q-39xc-vjpf', '    - GHSA-fc5q-39xc-vjpf\n    - GHSA-xxxx-yyyy-zzzz')
  assert.deepEqual(lapsedAdvisoryExceptions(unrecorded, '2026-11-18', advisoryExceptions),
    ['GHSA-xxxx-yyyy-zzzz has no recorded reason, expiry, and approver'])
  const unapproved = new Map([['GHSA-fc5q-39xc-vjpf', { expiry: '2026-12-09', reason: 'build tooling only' }]])
  assert.deepEqual(lapsedAdvisoryExceptions(auditYaml, '2026-11-18', unapproved),
    ['GHSA-fc5q-39xc-vjpf has no recorded reason, expiry, and approver'])
})

test('an advisory ignore list the check cannot read fails instead of passing', () => {
  assert.deepEqual(lapsedAdvisoryExceptions('auditConfig:\n  auditLevel: high\n', '2026-11-18', advisoryExceptions), [])
  assert.throws(() => lapsedAdvisoryExceptions('auditConfig:\n  ignoreGhsas: [GHSA-xxxx-yyyy-zzzz]\n', '2026-11-18'),
    /block list/)
  assert.throws(() => lapsedAdvisoryExceptions('auditConfig: { ignoreGhsas: [GHSA-xxxx-yyyy-zzzz] }\n', '2026-11-18'),
    /block list/)
})
