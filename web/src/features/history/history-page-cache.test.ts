import assert from 'node:assert/strict'
import test from 'node:test'
import type { HistoryEntrySummary, HistoryPage } from '@/api/types'
import { historyPageCacheKey, restoreHistoryPages, retainHistoryPages, resetHistoryPageCache } from './history-page-cache'
import { appendHistoryPage } from './history-pagination'

function page(overrides: Partial<HistoryPage> = {}): HistoryPage {
  return {
    repo_id: 'repo', generation: 'generation1', view_key: 'public', audience: 'public', feed: 'updates',
    head_oid: null, entries: [{ source_id: 'first' } as HistoryEntrySummary], next_cursor: 'older',
    ...overrides,
  }
}

test('navigation and entry selection restore accumulated history', () => {
  resetHistoryPageCache()
  const first = page()
  const key = historyPageCacheKey('viewer', first)
  const loaded = appendHistoryPage(first, page({ entries: [{ source_id: 'older' } as HistoryEntrySummary], next_cursor: null }), 'older')
  retainHistoryPages(key, loaded)
  assert.deepEqual(restoreHistoryPages(historyPageCacheKey('viewer', page()), page()), loaded)
  assert.equal(restoreHistoryPages(key, page()).next_cursor, null)
})

test('history generations, audiences, feeds, repositories and viewers are isolated', () => {
  resetHistoryPageCache()
  const first = page()
  const key = historyPageCacheKey('viewer', first)
  retainHistoryPages(key, { entries: [], next_cursor: null })
  for (const changed of [
    page({ generation: 'generation2' }), page({ audience: 'private' }), page({ feed: 'all' }),
    page({ repo_id: 'other' }), page({ view_key: 'private-view' }),
  ]) {
    assert.notEqual(historyPageCacheKey('viewer', changed), key)
    assert.equal(restoreHistoryPages(historyPageCacheKey('viewer', changed), changed).entries.length, 1)
  }
  assert.equal(restoreHistoryPages(historyPageCacheKey('other-viewer', first), first).entries.length, 1)
  assert.equal(restoreHistoryPages(null, first).entries.length, 1)
})
