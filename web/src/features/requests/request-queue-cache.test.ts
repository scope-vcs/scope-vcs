import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestQueueItemResponse, RequestQueuePageResponse } from '../../api/types.generated'
import { loadMoreRequestQueue, refreshRequestQueue, requestQueueResource, searchRequestQueue, type LoadRequestQueuePage } from './request-queue-cache'

const row = (id: string) => ({ request: { id } }) as RequestQueueItemResponse
const page = (ids: string[], cursor: string | null = null): RequestQueuePageResponse => ({ requests: ids.map(row), next_cursor: cursor, next_attention_at_unix: null })
const load: LoadRequestQueuePage = async (section, cursor, search) => page([`${section}-${search || 'all'}-${cursor || 'first'}`], cursor ? null : 'second')
const open = (key: string, version = '1', fetchPage = load) => requestQueueResource.load(key, version, (signal) => refreshRequestQueue(key, fetchPage, signal))

test('navigation reuses loaded pages and search in the same viewer/access scope', async () => {
  requestQueueResource.clear()
  let calls = 0
  const counted: LoadRequestQueuePage = (...args) => { calls++; return load(...args) }
  await open('repo/viewer/access', '1', counted)
  await loadMoreRequestQueue('repo/viewer/access', 'active', counted)
  const cached = await open('repo/viewer/access', '1', counted)
  assert.equal(calls, 4)
  assert.equal(cached.pages.active.requests.length, 2)
  await searchRequestQueue('repo/viewer/access', 'needle', counted)
  const searched = await open('repo/viewer/access', '1', counted)
  assert.equal(calls, 7)
  assert.equal(searched.query, 'needle')
  assert.equal(searched.pages.active.requests[0].request.id, 'active-needle-first')
  assert.equal((await open('repo/other-viewer/access')).query, '')
})

test('invalidation retains visible rows and refills loaded depth for the current search', async () => {
  requestQueueResource.clear()
  await open('refresh')
  await searchRequestQueue('refresh', 'needle', load)
  await loadMoreRequestQueue('refresh', 'active', load)
  const previous = requestQueueResource.peek('refresh')
  requestQueueResource.invalidate('refresh')
  let complete!: (value: RequestQueuePageResponse) => void
  const delayed: LoadRequestQueuePage = (section, cursor, query) => section === 'active' && !cursor
    ? new Promise((resolve) => { complete = resolve }) : load(section, cursor, query)
  const pending = open('refresh', '2', delayed)
  await Promise.resolve()
  assert.equal(requestQueueResource.peek('refresh'), previous)
  complete(page(['active-needle-new'], 'second'))
  const updated = await pending
  assert.equal(updated.query, 'needle')
  assert.deepEqual(updated.pages.active.requests.map(({ request }) => request.id), ['active-needle-new', 'active-needle-second'])
})

test('a late page cannot replace a newer search or a cleared viewer scope', async () => {
  requestQueueResource.clear()
  await open('race')
  let finish!: (value: RequestQueuePageResponse) => void
  const pending = loadMoreRequestQueue('race', 'active', () => new Promise((resolve) => { finish = resolve }))
  await Promise.resolve()
  await searchRequestQueue('race', 'new', load)
  finish(page(['obsolete']))
  await pending
  assert.equal(requestQueueResource.peek('race')?.query, 'new')
  const completions: ((value: RequestQueuePageResponse) => void)[] = []
  const late = searchRequestQueue('race', 'obsolete', () => new Promise((resolve) => { completions.push(resolve) }))
  await Promise.resolve()
  requestQueueResource.clear()
  for (const complete of completions) complete(page(['obsolete']))
  await late
  // Clearing detaches all outstanding attempts; no late result can recreate rows.
  assert.equal(requestQueueResource.peek('race'), null)
})

test('refresh failure leaves valid data available for retry', async () => {
  requestQueueResource.clear()
  const previous = await open('error')
  requestQueueResource.invalidate('error')
  await assert.rejects(open('error', '2', async () => { throw new Error('offline') }), /offline/)
  assert.equal(requestQueueResource.peek('error'), previous)
  assert.match(String(requestQueueResource.getSnapshot('error').error), /offline/)
  await open('error', '2')
  assert.equal(requestQueueResource.getSnapshot('error').error, null)
})
