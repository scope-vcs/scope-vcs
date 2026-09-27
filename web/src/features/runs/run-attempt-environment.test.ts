import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { RunEnvironmentPanel } from './run-attempt-environment'
import type { RepositoryRunCacheResponse } from '@/api/types.generated'

const image = `registry/scope@sha256:${'c'.repeat(64)}`
const caches: RepositoryRunCacheResponse[] = [
  {
    name: 'cargo',
    path: '/scope/cache/cargo',
    observation: {
      workflow_path: '/.scope/runs/checks.yml',
      job_key: 'backend',
      identity_digest: 'a'.repeat(64),
      preparation: { kind: 'exact' },
      key_ms: 2,
      metadata_ms: 3,
      size_bytes: 1_024 ** 2,
      download_verify_ms: 4,
      sync_ms: 1,
      extraction_ms: 2,
      prepare_ms: 12,
      final_state: 'ready',
      finalize_ms: 8,
    },
  },
  {
    name: 'target',
    path: '/workspace/target',
    observation: {
      workflow_path: '/.scope/runs/checks.yml',
      job_key: 'backend',
      identity_digest: 'b'.repeat(64),
      preparation: { kind: 'cold', reason: 'metadata-missing' },
      key_ms: 1,
      metadata_ms: 2,
      size_bytes: 0,
      download_verify_ms: 0,
      sync_ms: 0,
      extraction_ms: 0,
      prepare_ms: 3,
      final_state: 'pending',
      finalize_ms: null,
    },
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
})
