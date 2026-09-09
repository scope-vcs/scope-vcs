import assert from 'node:assert/strict'
import test from 'node:test'
import type { RepoFileContent } from '@/api/types'
import {
  repoFileResource,
  repoFileCacheKey,
} from './repo-file-cache'

function textFile(path: string, oid: string, text: string): RepoFileContent {
  return {
    content: { kind: 'text', text },
    oid,
    path,
    size_bytes: text.length,
    visibility: 'Public',
  }
}

test('keys file entries by repository version, audience and normalized path', () => {
  const base = {
    audience: 'public' as const,
    changeVersion: 3,
    path: 'README.html',
    repoId: 'repo-1',
  }

  assert.notEqual(
    repoFileCacheKey(base),
    repoFileCacheKey({ ...base, audience: 'private' }),
  )
  assert.equal(repoFileCacheKey(base), repoFileCacheKey({ ...base, path: '/README.html' }))
  assert.notEqual(
    repoFileCacheKey(base),
    repoFileCacheKey({ ...base, changeVersion: 4 }),
  )
  assert.notEqual(
    repoFileCacheKey(base),
    repoFileCacheKey({ ...base, path: 'another.ts' }),
  )
})

test('evicts old entries at the entry limit', () => {
  repoFileResource.clear()
  for (let index = 0; index < 40; index += 1) {
    repoFileResource.write(`file-${index}`, textFile(`${index}.ts`, `${index}`, 'x'))
  }

  assert.equal(repoFileResource.stats().entries, 32)
  assert.equal(repoFileResource.read('file-0'), null)
  assert.equal(repoFileResource.read('file-39')?.path, '39.ts')
})

test('evicts large source entries at the byte limit', () => {
  repoFileResource.clear()
  const sixMiBOfText = 'x'.repeat(3 * 1024 * 1024)
  for (let index = 0; index < 6; index += 1) {
    repoFileResource.write(
      `large-${index}`,
      textFile(`${index}.txt`, `${index}`, sixMiBOfText),
    )
  }

  const stats = repoFileResource.stats()
  assert.ok(stats.entries < 6)
  assert.ok(stats.totalWeight <= 24 * 1024 * 1024)
})
