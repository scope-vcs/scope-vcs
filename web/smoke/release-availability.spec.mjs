import assert from 'node:assert/strict'
import { existsSync, readFileSync } from 'node:fs'
import { test } from 'node:test'
import { baseUrl } from './browser-smoke.mjs'

const availabilityCore = new URL('../../deploy/scripts/release-availability-core.mjs', import.meta.url)
const deploymentManifest = new URL('../../.github/deployment-services.json', import.meta.url)
const skip = existsSync(availabilityCore) && existsSync(deploymentManifest)
  ? false
  : 'needs deploy/scripts and the deployment manifest, which this checkout does not include'

test('the served landing page satisfies the production availability gate', { skip }, async () => {
  const { availabilityTargets, probeTarget } = await import(availabilityCore)
  const manifest = JSON.parse(readFileSync(deploymentManifest, 'utf8'))
  const target = availabilityTargets({ ...manifest.releaseAvailability.production, webOrigin: baseUrl })
    .find(({ name }) => name === 'public-homepage')
  const result = await probeTarget(target, { timeoutMs: 5000 })
  assert.equal(result.ok, true, JSON.stringify(result))
})
