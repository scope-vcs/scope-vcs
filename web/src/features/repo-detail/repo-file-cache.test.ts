import assert from 'node:assert/strict'
import test from 'node:test'
import { repoFileCacheKey } from './repo-file-cache'

test('keys file entries by repository version, view and normalized path', () => {
  const base = {
    scope: 'viewer-a',
    view: 'public' as const,
    contentVersion: 3,
    path: 'README.html',
    repoId: 'repo-1',
  }

  assert.notEqual(
    repoFileCacheKey(base),
    repoFileCacheKey({ ...base, scope: 'viewer-b' }),
  )
  assert.notEqual(
    repoFileCacheKey(base),
    repoFileCacheKey({ ...base, view: 'private' }),
  )
  assert.equal(repoFileCacheKey(base), repoFileCacheKey({ ...base, path: '/README.html' }))
  assert.notEqual(
    repoFileCacheKey(base),
    repoFileCacheKey({ ...base, contentVersion: 4 }),
  )
  assert.notEqual(
    repoFileCacheKey(base),
    repoFileCacheKey({ ...base, path: 'another.ts' }),
  )
})
