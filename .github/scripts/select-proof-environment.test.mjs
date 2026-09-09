import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'
import { selectProofEnvironment } from './select-proof-environment.mjs'

const manifest = JSON.parse(readFileSync(new URL('../deployment-services.json', import.meta.url)))

test('release-proof selects the dedicated release environment', () => {
  const changed = structuredClone(manifest)
  changed.railway.staging.environmentName = 'release-proof'
  assert.deepEqual(selectProofEnvironment(changed, 'release-proof'), changed)
})

test('isolated proof selects its own environment and domains without changing production', () => {
  const result = selectProofEnvironment(manifest, 'media-proof')
  assert.equal(result.railway.staging.environmentName, 'media-proof')
  assert.notEqual(result.railway.staging.environmentId, manifest.railway.staging.environmentId)
  assert.equal(result.railway.environmentId, manifest.railway.environmentId)
  assert.equal(result.mediaResources.staging.gatewayDomain, 'scope-media-media-proof.up.railway.app')
  for (const key of ['apiDomain', 'cacheDomain', 'routerDomain', 'webDomain']) {
    assert.notEqual(result.railway.staging[key], manifest.railway.staging[key])
  }
})

test('unreviewed, production, and shared-staging aliases cannot become isolated targets', () => {
  for (const name of ['production', 'staging', 'unknown', '__proto__']) {
    assert.throws(() => selectProofEnvironment(manifest, name), /Unknown proof environment/)
  }
  for (const environmentId of [manifest.railway.environmentId, manifest.railway.staging.environmentId]) {
    const changed = structuredClone(manifest)
    changed.railway.proofEnvironments['media-proof'].environmentId = environmentId
    assert.throws(() => selectProofEnvironment(changed, 'media-proof'), /must differ/)
  }
})

test('an isolated target must retain its reviewed environment name', () => {
  const changed = structuredClone(manifest)
  changed.railway.proofEnvironments['media-proof'].environmentName = 'another-proof'
  assert.throws(() => selectProofEnvironment(changed, 'media-proof'), /name does not match/)
})

test('release proof cannot be redirected to experimental staging', () => {
  const changed = structuredClone(manifest)
  changed.railway.staging.environmentName = 'staging'
  assert.throws(() => selectProofEnvironment(changed, 'release-proof'), /must target release-proof/)
})
