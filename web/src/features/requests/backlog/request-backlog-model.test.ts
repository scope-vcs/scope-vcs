import assert from 'node:assert/strict'
import test from 'node:test'
import {
  closeBacklogRequests,
  requestBacklogQuery,
  requestBacklogQueryFor,
  requestBacklogRouteSearch,
} from './request-backlog-model'

test('closing several requests reports each one that failed and finishes the rest', async () => {
  const attempted: string[] = []
  const failures = await closeBacklogRequests(['a', 'b', 'c', 'd', 'e'], async (id) => {
    attempted.push(id)
    if (id === 'b') throw new Error('request is already closed')
    if (id === 'd') throw new Error(' ')
  }, 2)
  assert.deepEqual([...attempted].sort(), ['a', 'b', 'c', 'd', 'e'])
  assert.deepEqual([...failures], [
    ['b', 'request is already closed'],
    ['d', 'Could not close this request.'],
  ])
})

test('the backlog link keeps only the search, filter, and sort that differ from the defaults', () => {
  assert.deepEqual(
    requestBacklogRouteSearch(requestBacklogQuery({ q: ' #142 ', working: 'me', sort: 'newest' })),
    { q: '#142', working: 'me', sort: 'newest' },
  )
  assert.equal(requestBacklogQuery({ q: 142 }).search, '142')
  assert.deepEqual(
    requestBacklogRouteSearch(requestBacklogQuery({ q: '  ', working: 'everyone', sort: 'oldest' })),
    { q: undefined, working: undefined, sort: undefined },
  )
})

test('a signed-out visitor asking for their own work gets everyone\'s, with the rest of the query kept', () => {
  const mine = requestBacklogQuery({ q: 'login', working: 'me', sort: 'newest' })
  assert.deepEqual(requestBacklogQueryFor(mine, true), { search: 'login', working: 'anyone', sort: 'newest' })
  assert.equal(requestBacklogQueryFor(mine, false), mine)
  const untaken = requestBacklogQuery({ working: 'nobody' })
  assert.equal(requestBacklogQueryFor(untaken, true), untaken)
})
