import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'
import { availabilityTargets, probeTarget } from '../../deploy/scripts/release-availability-core.mjs'
import { baseUrl } from './browser-smoke.mjs'

test('the served landing page satisfies the production availability gate', async () => {
  const manifest = JSON.parse(readFileSync(new URL('../../.github/deployment-services.json', import.meta.url), 'utf8'))
  const target = availabilityTargets({ ...manifest.releaseAvailability.production, webOrigin: baseUrl })
    .find(({ name }) => name === 'public-homepage')
  const result = await probeTarget(target, { timeoutMs: 5000 })
  assert.equal(result.ok, true, JSON.stringify(result))
})
