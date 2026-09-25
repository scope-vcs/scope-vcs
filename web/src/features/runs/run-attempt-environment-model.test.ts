import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import {
  cacheSetupLabel,
  cacheSizeLabel,
  cacheStateClass,
  cacheStateDetail,
  cacheStateLabel,
  cacheTimingLabel,
  cachesNeedAttention,
  pinnedImageLabel,
} from './run-attempt-environment-model'
import type { RepositoryRunCacheResponse } from '@/api/types.generated'

const caches: RepositoryRunCacheResponse[] = [
  {
    name: 'cargo',
    path: '/scope/cache/cargo',
    observation: {
      workflow_path: '/.scope/runs/checks.yml',
      job_key: 'backend',
      identity_digest: 'a'.repeat(64),
      preparation: { kind: 'exact' },
      key_ms: 10,
      metadata_ms: 20,
      size_bytes: 512 * 1_024 * 1_024,
      download_verify_ms: 80,
      sync_ms: 40,
      extraction_ms: 50,
      prepare_ms: 200,
      final_state: 'ready',
      finalize_ms: 100,
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
      key_ms: 100,
      metadata_ms: 1_000,
      size_bytes: 0,
      download_verify_ms: 0,
      sync_ms: 0,
      extraction_ms: 0,
      prepare_ms: 1_100,
      final_state: 'pending',
      finalize_ms: null,
    },
  },
  {
    name: 'cargo-target',
    path: '/scope/cache/cargo-target',
    observation: {
      workflow_path: '/.scope/runs/checks.yml',
      job_key: 'backend',
      identity_digest: 'c'.repeat(64),
      preparation: { kind: 'compatible' },
      key_ms: 100,
      metadata_ms: 100,
      size_bytes: 1_024 ** 3,
      download_verify_ms: 300,
      sync_ms: 100,
      extraction_ms: 300,
      prepare_ms: 900,
      final_state: 'ready',
      finalize_ms: 50,
    },
  },
  {
    name: 'playwright',
    path: '/root/.cache/ms-playwright',
    observation: null,
  },
]

describe('run attempt environment model', () => {
  it('flags a cold or unreported cache for the collapsed control', () => {
    assert.equal(cachesNeedAttention(caches), true)
    assert.equal(cachesNeedAttention([caches[0]!, caches[2]!]), false)
    assert.equal(cachesNeedAttention([caches[3]!]), true)
    assert.equal(cachesNeedAttention([]), false)
  })

  it('keeps missing metadata distinct from a missing report', () => {
    assert.equal(cacheStateLabel(caches[1]!), 'cold')
    assert.equal(cacheStateDetail(caches[1]!), 'No reusable entry for this identity')
    assert.equal(cacheStateLabel(caches[3]!), 'not reported')
    assert.equal(
      cacheStateDetail(caches[3]!),
      'Cache facts were not reported for this attempt.',
    )
    assert.equal(cacheStateDetail(caches[0]!), null)
    assert.equal(cacheStateClass(caches[0]!), 'text-success')
    assert.equal(cacheStateClass(caches[1]!), 'text-warning')
    assert.equal(cacheStateClass(caches[2]!), 'text-success')
    assert.equal(cacheStateClass(caches[3]!), 'text-muted-foreground')
  })

  it('reports size and total preparation time only when observed', () => {
    assert.equal(cacheSizeLabel(caches[0]!), '512.0 MB')
    assert.equal(cacheTimingLabel(caches[0]!), '200ms')
    assert.equal(cacheTimingLabel(caches[1]!), '1.1s')
    assert.equal(cacheSizeLabel(caches[3]!), null)
    assert.equal(cacheTimingLabel(caches[3]!), null)
    assert.equal(cacheSetupLabel({ authorization_ms: 75, wall_ms: 2_200 }), 'Set up in 2.2s')
    assert.equal(cacheSetupLabel(null), null)
  })

  it('formats immutable image identity without the registry noise', () => {
    assert.equal(
      pinnedImageLabel(`registry/scope@sha256:${'c'.repeat(64)}`),
      `sha256:${'c'.repeat(12)}`,
    )
    assert.equal(pinnedImageLabel(null), 'Image not pinned yet')
  })
})
