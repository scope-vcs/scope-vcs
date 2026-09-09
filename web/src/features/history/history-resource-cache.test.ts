import assert from 'node:assert/strict'
import test from 'node:test'
import type { ReviewFileDiff } from '@/api/types'
import {
  historyEntryCacheKey,
  historyDiffCacheKey,
  historyEntryDiffCacheKey,
  historyResourceCacheStats,
  historyDiffResource,
  readHistoryDiffScroll,
  resetHistoryResourceCache,
  writeHistoryDiffScroll,
} from './history-resource-cache'

function diff(path: string, text = 'content'): ReviewFileDiff {
  return {
    kind: 'Modified',
    new_mode: '100644',
    old_mode: '100644',
    path,
    presentation: { html: text, kind: 'html' },
  }
}

test('keys resources by immutable audience-aware identities', () => {
  const commitBase = {
    audience: 'public' as const,
    entry: 'c1',
    generation: 'generation-1',
    repoId: 'scope/demo',
    viewKey: 'public',
  }
  assert.notEqual(
    historyEntryCacheKey(commitBase),
    historyEntryCacheKey({ ...commitBase, audience: 'private' }),
  )
  assert.notEqual(
    historyEntryCacheKey(commitBase),
    historyEntryCacheKey({ ...commitBase, generation: 'generation-2' }),
  )

  const diffBase = {
    ...commitBase,
    commit: commitBase.entry,
    newOid: 'new',
    oldOid: 'old',
    path: '/README.md',
  }
  assert.notEqual(
    historyDiffCacheKey(diffBase),
    historyDiffCacheKey({ ...diffBase, newOid: 'newer' }),
  )
  assert.notEqual(
    historyDiffCacheKey(diffBase),
    historyDiffCacheKey({ ...diffBase, path: '/other.md' }),
  )
})

test('bounds diff entries with least-recently-used eviction', () => {
  resetHistoryResourceCache()
  for (let index = 0; index < 30; index += 1) {
    historyDiffResource.write(`diff-${index}`, diff(`/${index}.txt`))
  }

  assert.equal(historyResourceCacheStats().diffs, 20)
  assert.equal(historyDiffResource.read('diff-0'), null)
  assert.equal(historyDiffResource.read('diff-29')?.path, '/29.txt')
})

test('keeps diff scroll state with its bounded cache entry', () => {
  resetHistoryResourceCache()
  historyDiffResource.write('readme', diff('/README.md'))
  writeHistoryDiffScroll('readme', 420)
  historyDiffResource.write('readme', diff('/README.md', 'updated'))
  assert.equal(readHistoryDiffScroll('readme'), 420)

  for (let index = 0; index < 20; index += 1) {
    historyDiffResource.write(`diff-${index}`, diff(`/${index}.txt`))
  }
  assert.equal(readHistoryDiffScroll('readme'), 0)
})

test('evicts large text diffs at the byte budget', () => {
  resetHistoryResourceCache()
  const largeText = 'x'.repeat(3 * 1024 * 1024)
  for (let index = 0; index < 6; index += 1) {
    historyDiffResource.write(`large-${index}`, diff(`/${index}.txt`, largeText))
  }

  const stats = historyResourceCacheStats()
  assert.ok(stats.diffs < 6)
  assert.ok(stats.diffBytes <= 32 * 1024 * 1024)
})

test('isolates content and exact visibility preview caches for the same file and blobs', () => {
  const base = { audience: 'public' as const, entry: 'push-1', generation: 'g1', repoId: 'scope/demo', viewKey: 'public', path: '/same.ts', oldOid: null, newOid: 'blob' }
  assert.notEqual(historyEntryDiffCacheKey(base), historyEntryDiffCacheKey({ ...base, visibilityChange: 'first' }))
  assert.notEqual(historyEntryDiffCacheKey({ ...base, visibilityChange: 'first' }), historyEntryDiffCacheKey({ ...base, visibilityChange: 'second' }))
})
