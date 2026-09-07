import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestListItem } from '@/api/types'
import { requestQueueViewReducer, type RequestQueuePages } from './request-list-model'
import { restoreRequestQueue, retainRequestQueue, resetRequestQueueCache } from './request-queue-cache'

function pages(): RequestQueuePages {
  return {
    open: { requests: [{ id: 'first' } as RequestListItem], next_cursor: 'older' },
    closed: { requests: [], next_cursor: null },
    your_work: { requests: [], next_cursor: null },
  }
}

function paginated() {
  return requestQueueViewReducer(restoreRequestQueue(null, pages()), {
    type: 'load_succeeded', generation: 0, section: 'open',
    page: { requests: [{ id: 'older' } as RequestListItem], next_cursor: null },
  })
}

test('navigation restores loaded requests across equivalent loader snapshots', () => {
  resetRequestQueueCache()
  retainRequestQueue('repo/viewer/version', paginated())
  const restored = restoreRequestQueue('repo/viewer/version', pages())
  assert.deepEqual(restored.pages.open.requests.map(({ id }) => id), ['first', 'older'])
  assert.equal(restored.pages.open.next_cursor, null)
})

test('navigation retains committed search results but releases abandoned operations', () => {
  resetRequestQueueCache()
  const searched = requestQueueViewReducer(paginated(), {
    type: 'search_succeeded', generation: 0, query: 'needle',
    open: { requests: [{ id: 'matching' } as RequestListItem], next_cursor: null },
    closed: { requests: [], next_cursor: null },
  })
  retainRequestQueue('search', { ...searched, searchDraft: 'needle', searching: true, loadingSection: 'open' })
  const restored = restoreRequestQueue('search', pages())
  assert.equal(restored.searchQuery, 'needle')
  assert.equal(restored.searchDraft, 'needle')
  assert.deepEqual(restored.pages.open.requests.map(({ id }) => id), ['matching'])
  assert.equal(restored.searching, false)
  assert.equal(restored.loadingSection, null)
})

test('new snapshots replace stale request rows and version or viewer scopes never reuse them', () => {
  resetRequestQueueCache()
  retainRequestQueue('viewer/version1', paginated())
  const changed = pages()
  changed.open.requests = []
  assert.equal(restoreRequestQueue('viewer/version1', changed).pages.open.requests.length, 0)
  assert.equal(restoreRequestQueue('viewer/version2', pages()).pages.open.requests.length, 1)
  assert.equal(restoreRequestQueue('other-viewer/version1', pages()).pages.open.requests.length, 1)
  assert.equal(restoreRequestQueue(null, pages()).pages.open.requests.length, 1)
})

test('retention evicts older queues and oversized payloads', () => {
  resetRequestQueueCache()
  for (let index = 0; index < 13; index++) retainRequestQueue(String(index), paginated())
  assert.equal(restoreRequestQueue('0', pages()).pages.open.requests.length, 1)
  assert.equal(restoreRequestQueue('12', pages()).pages.open.requests.length, 2)
  retainRequestQueue('oversized', { ...paginated(), searchDraft: 'x'.repeat(3 * 1024 * 1024) })
  assert.equal(restoreRequestQueue('oversized', pages()).pages.open.requests.length, 1)
})
