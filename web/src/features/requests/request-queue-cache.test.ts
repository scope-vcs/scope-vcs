import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestQueueItemResponse, RequestQueuePageResponse } from '../../api/types.generated'
import { loadMoreRequestQueue, refreshRequestQueue, requestQueueResource, type LoadRequestQueuePage } from './request-queue-cache'

const row = (id: string) => ({ request: { id } }) as RequestQueueItemResponse
const page = (ids: string[], cursor: string | null = null): RequestQueuePageResponse => ({ requests: ids.map(row), next_cursor: cursor, next_attention_at_unix: null })
const load: LoadRequestQueuePage = async (section, cursor) => page([`${section}-${cursor || 'first'}`], cursor ? null : 'second')
const open = (key: string, version = '1', fetchPage = load) => requestQueueResource.load(key, version, (signal) => refreshRequestQueue(key, fetchPage, signal))

test('navigation reuses loaded pages in the same viewer/access scope', async () => {
  requestQueueResource.clear()
  let calls = 0
  const counted: LoadRequestQueuePage = (...args) => { calls++; return load(...args) }
  await open('repo/viewer/access', '1', counted)
  await loadMoreRequestQueue('repo/viewer/access', 'active', counted)
  const cached = await open('repo/viewer/access', '1', counted)
  assert.equal(calls, 5)
  assert.equal(cached.pages.active.requests.length, 2)
})

test('invalidation retains visible rows and refills loaded depth', async () => {
  requestQueueResource.clear()
  await open('refresh')
  await loadMoreRequestQueue('refresh', 'active', load)
  const previous = requestQueueResource.peek('refresh')
  requestQueueResource.invalidate('refresh')
  let complete!: (value: RequestQueuePageResponse) => void
  const delayed: LoadRequestQueuePage = (section, cursor) => section === 'active' && !cursor
    ? new Promise((resolve) => { complete = resolve }) : load(section, cursor)
  const pending = open('refresh', '2', delayed)
  await Promise.resolve()
  assert.equal(requestQueueResource.peek('refresh'), previous)
  complete(page(['active-new'], 'second'))
  const updated = await pending
  assert.deepEqual(updated.pages.active.requests.map(({ request }) => request.id), ['active-new', 'active-second'])
})

test('a late page cannot repopulate a cleared viewer scope', async () => {
  requestQueueResource.clear()
  await open('race')
  let finish!: (value: RequestQueuePageResponse) => void
  const pending = loadMoreRequestQueue('race', 'active', () => new Promise((resolve) => { finish = resolve }))
  await Promise.resolve()
  requestQueueResource.clear()
  finish(page(['obsolete']))
  await pending
  assert.equal(requestQueueResource.peek('race'), null)
})
