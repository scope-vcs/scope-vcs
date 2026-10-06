import assert from 'node:assert/strict'
import test from 'node:test'
import type { RepoContent } from '@/api/types'
import {
  repoContentResource,
  repoContentCacheKey,
} from './repo-content-cache'

const content: RepoContent = {
  clone_remote_url: 'https://example.com/owner/repo.git',
  files: [],
}

test('keys repository content by version and view', () => {
  const base = {
    scope: 'viewer-a',
    view: 'public' as const,
    contentVersion: 3,
    repoId: 'repo-1',
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
})

test('bounds repository content entries', () => {
  repoContentResource.clear()
  for (let index = 0; index < 10; index += 1) {
    repoContentResource.write(`repo-${index}`, content)
  }

  assert.equal(repoContentResource.stats().entries, 8)
  assert.equal(repoContentResource.read('repo-0'), null)
  assert.equal(repoContentResource.read('repo-9'), content)
})
