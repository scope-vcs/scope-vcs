import assert from 'node:assert/strict'
import test from 'node:test'
import type { RequestQueueItemResponse, RequestQueuePageResponse } from '../../api/types.generated'
import { appendQueuePage, nextRequestAttentionAt } from './request-list-model'

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
