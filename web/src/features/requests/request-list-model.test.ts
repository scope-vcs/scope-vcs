import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestQueueItemResponse, RequestQueuePageResponse } from '../../api/types.generated'
import { appendQueuePage, nextRequestAttentionAt, requestQueueReadableBy } from './request-list-model'

const row = (id: string, title = id) => ({ request: { id, title } }) as RequestQueueItemResponse
const page = (requests: RequestQueueItemResponse[], expiry: number | null = null): RequestQueuePageResponse => ({ requests, next_cursor: null, next_attention_at_unix: expiry })

test('pagination preserves order and replaces repeated rows with current server facts', () => {
  const original = page([row('first'), row('second')])
  const next = appendQueuePage(original, page([row('second', 'Updated'), row('third')]))
  assert.deepEqual(next.requests.map(({ request }) => [request.id, request.title]), [['first', 'first'], ['second', 'Updated'], ['third', 'third']])
  assert.equal(original.requests[1].request.title, 'second')
})

test('attention expiry uses server-wide next time, including requests outside loaded rows', () => {
  assert.equal(nextRequestAttentionAt({ active: page([], 400), unclaimed: page([], 300), set_aside: page([], null), done: page([], null) }), 300)
  assert.equal(nextRequestAttentionAt({ active: page([]), unclaimed: page([]), set_aside: page([]), done: page([]) }), null)
})

test('viewing as a narrower view keeps only the requests that view may read', () => {
  const inView = (id: string, view: string) => ({ request: { id, title: id, view } }) as RequestQueueItemResponse
  const pages = {
    active: { ...page([inView('a', 'public'), inView('b', 'private')]), next_cursor: 'more' },
    unclaimed: page([inView('c', 'agent')]),
    set_aside: page([]),
    done: page([inView('d', 'public')]),
  }
  const readable = requestQueueReadableBy(pages, (view) => view === 'public' || view === 'agent')
  assert.deepEqual(readable.active.requests.map(({ request }) => request.id), ['a'])
  assert.equal(readable.active.next_cursor, 'more')
  assert.deepEqual(readable.unclaimed.requests.map(({ request }) => request.id), ['c'])
  assert.deepEqual(readable.done.requests.map(({ request }) => request.id), ['d'])
})
