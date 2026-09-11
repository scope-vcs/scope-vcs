import assert from 'node:assert/strict'
import test from 'node:test'
import type {
  RequestQueuePages,
  RequestQueueViewAction,
} from './request-list-model'
import { dispatchRequestQueue, openRequestQueue, requestQueueResource, resetRequestQueueCache } from './request-queue-cache'
import type { RequestListItemResponse } from '@/api/types.generated'

function pages(): RequestQueuePages {
  return {
    open: { requests: [{ id: 'first' } as RequestListItemResponse], next_cursor: 'older' },
    closed: { requests: [], next_cursor: null },
    your_work: { requests: [], next_cursor: null },
  }
}

function paginate(key: string) {
  openRequestQueue(key, pages())
  dispatch(key, {
    type: 'load_succeeded', generation: 0, section: 'open',
    page: { requests: [{ id: 'older' } as RequestListItemResponse], next_cursor: null },
  })
}

test('navigation reuses loaded requests across equivalent loader snapshots', () => {
  resetRequestQueueCache()
  paginate('repo/viewer/version')
  const reopened = openRequestQueue('repo/viewer/version', pages())
  assert.deepEqual(reopened.pages.open.requests.map(({ id }) => id), ['first', 'older'])
  assert.equal(reopened.pages.open.next_cursor, null)
})

test('search operations remain owned by the resource while a page is closed', () => {
  resetRequestQueueCache()
  paginate('search')
  dispatch('search', { type: 'search_draft_changed', value: 'needle' })
  dispatch('search', { type: 'search_started', generation: 0 })
  assert.equal(openRequestQueue('search', pages()).searching, true)
  dispatch('search', {
    type: 'search_succeeded', generation: 0, query: 'needle',
    open: { requests: [{ id: 'matching' } as RequestListItemResponse], next_cursor: null },
    closed: { requests: [], next_cursor: null },
  })
  const reopened = openRequestQueue('search', pages())
  assert.equal(reopened.searchQuery, 'needle')
  assert.equal(reopened.searchDraft, 'needle')
  assert.deepEqual(reopened.pages.open.requests.map(({ id }) => id), ['matching'])
  assert.equal(reopened.searching, false)
})

test('changed snapshots invalidate older operations and scope changes isolate rows', () => {
  resetRequestQueueCache()
  paginate('viewer/version1')
  const changed = pages()
  changed.open.requests = []
  openRequestQueue('viewer/version1', changed)
  dispatch('viewer/version1', {
    type: 'load_succeeded', generation: 0, section: 'open', page: pages().open,
  })
  assert.equal(requestQueueResource.peek('viewer/version1')?.pages.open.requests.length, 0)
  assert.equal(openRequestQueue('viewer/version2', pages()).pages.open.requests.length, 1)
  assert.equal(openRequestQueue('other-viewer/version1', pages()).pages.open.requests.length, 1)
})

test('retention evicts older queues and keeps oversized results only while subscribed', () => {
  resetRequestQueueCache()
  for (let index = 0; index < 13; index++) paginate(String(index))
  assert.equal(openRequestQueue('0', pages()).pages.open.requests.length, 1)
  assert.equal(openRequestQueue('12', pages()).pages.open.requests.length, 2)
  const leave = requestQueueResource.subscribe('12', () => {})
  dispatch('12', { type: 'search_draft_changed', value: 'x'.repeat(3 * 1024 * 1024) })
  assert.equal(requestQueueResource.peek('12')?.pages.open.requests.length, 2)
  leave()
  assert.equal(requestQueueResource.peek('12'), null)
})

function dispatch(key: string, action: RequestQueueViewAction) {
  dispatchRequestQueue(key, action, requestQueueResource.peek(key)!.owner)
}

test('late queue responses cannot update a replacement resource with the same generation', () => {
  resetRequestQueueCache()
  const original = openRequestQueue('request', pages())
  resetRequestQueueCache()
  openRequestQueue('request', pages())
  dispatchRequestQueue('request', {
    type: 'load_succeeded', generation: 0, section: 'open',
    page: { requests: [{ id: 'late' } as RequestListItemResponse], next_cursor: null },
  }, original.owner)
  assert.deepEqual(requestQueueResource.peek('request')?.pages.open.requests.map(({ id }) => id), ['first'])
})
