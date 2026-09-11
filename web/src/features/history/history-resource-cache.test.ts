import assert from 'node:assert/strict'
import test from 'node:test'
import type { ReviewFileDiff } from '@/api/types'
import {
  historyDiffCacheKey,
  historyEntryDiffCacheKey,
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

test('isolates content and exact visibility preview caches for the same file and blobs', () => {
  const base = { scope: 'viewer-a', audience: 'public' as const, entry: 'push-1', generation: 'g1', repoId: 'scope/demo', viewKey: 'public', path: '/same.ts', oldOid: null, newOid: 'blob' }
  assert.notEqual(historyEntryDiffCacheKey(base), historyEntryDiffCacheKey({ ...base, visibilityChange: 'first' }))
  assert.notEqual(historyEntryDiffCacheKey({ ...base, visibilityChange: 'first' }), historyEntryDiffCacheKey({ ...base, visibilityChange: 'second' }))
})

test('same-viewer history diffs reuse data while viewer and access changes load independently', async () => {
  resetHistoryResourceCache()
  const base = { scope: 'viewer-a:member', audience: 'private' as const, commit: 'c1', generation: 'g1', newOid: 'new', oldOid: 'old', path: '/README.md', repoId: 'scope/demo', viewKey: 'private' }
  let loads = 0
  const load = async () => { loads += 1; return diff(base.path) }
  const key = historyDiffCacheKey(base)
  await historyDiffResource.load(key, '', load)
  await historyDiffResource.load(historyDiffCacheKey({ ...base }), '', load)
  assert.equal(loads, 1)
  for (const change of [
    { scope: 'viewer-b:member' }, { scope: 'viewer-a:public', audience: 'public' as const },
    { generation: 'g2' }, { newOid: 'newer' }, { path: '/other.md' },
  ]) await historyDiffResource.load(historyDiffCacheKey({ ...base, ...change }), '', load)
  assert.equal(loads, 6)
})
