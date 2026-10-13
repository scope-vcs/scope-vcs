import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { RunEnvironmentPanel } from './run-attempt-environment'
import {
  cacheSetupLabel,
  cacheSizeLabel,
  cacheStateClass,
  cacheTimingLabel,
  cachesNeedAttention,
  pinnedImageLabel,
} from './run-attempt-environment-model'
import type { RepositoryRunCacheResponse } from '@/api/types.generated'

const image = `registry/scope@sha256:${'c'.repeat(64)}`

function observed(
  identity: string,
  preparation: NonNullable<RepositoryRunCacheResponse['observation']>['preparation'],
  timing: { key_ms: number; metadata_ms: number; size_bytes: number; prepare_ms: number },
): RepositoryRunCacheResponse['observation'] {
  return {
    workflow_path: '/.scope/runs/checks.yml',
    job_key: 'backend',
    identity_digest: identity.repeat(64),
    preparation,
    download_verify_ms: 0,
    sync_ms: 0,
    extraction_ms: timing.prepare_ms - timing.key_ms - timing.metadata_ms,
    final_state: 'ready',
    finalize_ms: 8,
    ...timing,
  }
}

const caches: RepositoryRunCacheResponse[] = [
  {
    name: 'cargo',
    path: '/scope/cache/cargo',
    observation: observed('a', { kind: 'exact' }, { key_ms: 2, metadata_ms: 3, size_bytes: 1_024 ** 2, prepare_ms: 12 }),
  },
  {
    name: 'target',
    path: '/workspace/target',
    observation: observed('b', { kind: 'cold', reason: 'metadata-missing' }, { key_ms: 100, metadata_ms: 1_000, size_bytes: 0, prepare_ms: 1_100 }),
  },
  {
    name: 'cargo-target',
    path: '/scope/cache/cargo-target',
    observation: observed('c', { kind: 'compatible' }, { key_ms: 1, metadata_ms: 1, size_bytes: 1_024, prepare_ms: 900 }),
  },
  {
    name: 'playwright',
    path: '/root/.cache/ms-playwright',
    observation: null,
  },
]

describe('run attempt environment', () => {
  it('lists each cache with its result, size and time, then the image', () => {
    const html = renderToStaticMarkup(createElement(RunEnvironmentPanel, {
      caches,
      cacheSetup: {
        authorization_ms: 3,
        wall_ms: 20,
      },
      id: 'environment',
      pinnedContainerImage: image,
    }))

    assert.match(html, /aria-label="Execution environment"/)
    assert.match(html, /cargo<\/span>.*exact.*1\.0 MB.*12ms/)
    assert.match(html, /title="No reusable entry for this identity"[^>]*>cold/)
    assert.match(html, /title="Cache facts were not reported for this attempt\."[^>]*>not reported/)
    assert.match(html, new RegExp(`title="${image}"`))
    assert.match(html, /Set up in 20ms/)
    assert.doesNotMatch(html, /authorized|finalize|key 2ms|checks\.yml/)
    assert.doesNotMatch(html, /rounded|shadow/)
  })

  it('flags a cold or unreported cache for the collapsed control', () => {
    assert.equal(cachesNeedAttention(caches), true)
    assert.equal(cachesNeedAttention([caches[0]!, caches[2]!]), false)
    assert.equal(cachesNeedAttention([caches[3]!]), true)
    assert.equal(cachesNeedAttention([]), false)
  })

  it('styles results and labels only observed facts', () => {
    assert.deepEqual(caches.map(cacheStateClass), ['text-success', 'text-warning', 'text-success', 'text-muted-foreground'])
    assert.equal(cacheTimingLabel(caches[1]!), '1.1s')
    assert.equal(cacheSizeLabel(caches[3]!), null)
    assert.equal(cacheTimingLabel(caches[3]!), null)
    assert.equal(cacheSetupLabel({ authorization_ms: 75, wall_ms: 2_200 }), 'Set up in 2.2s')
    assert.equal(cacheSetupLabel(null), null)
  })

  it('formats immutable image identity without the registry noise', () => {
    assert.equal(pinnedImageLabel(image), `sha256:${'c'.repeat(12)}`)
    assert.equal(pinnedImageLabel(null), 'Image not pinned yet')
  })
})
