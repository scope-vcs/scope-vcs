import assert from 'node:assert/strict'
import test from 'node:test'
import { repoContentCacheKey } from './repo-content-cache'
import type { RepositoryVersion } from './repository-version'

test('keys repository content by version and view', () => {
  const base = {
    scope: 'viewer-a',
    view: 'public' as const,
    contentVersion: 3,
    repoId: 'repo-1',
    version: { kind: 'main' } as RepositoryVersion,
  }

  assert.notEqual(
    repoContentCacheKey(base),
    repoContentCacheKey({ ...base, scope: 'viewer-b' }),
  )
  assert.notEqual(
    repoContentCacheKey(base),
    repoContentCacheKey({ ...base, view: 'private' }),
  )
  assert.notEqual(
    repoContentCacheKey(base),
    repoContentCacheKey({ ...base, contentVersion: 4 }),
  )
  assert.notEqual(
    repoContentCacheKey({ ...base, version: { kind: 'request', id: 'one' } }),
    repoContentCacheKey({ ...base, version: { kind: 'update', id: 'one' } }),
  )
})
