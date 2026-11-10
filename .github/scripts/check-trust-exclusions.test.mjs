import assert from 'node:assert/strict'
import test from 'node:test'
import { expiredTrustExclusions, trustExclusions } from './check-trust-exclusions.mjs'

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
