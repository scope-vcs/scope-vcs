import assert from 'node:assert/strict'
import test from 'node:test'
import {
  openRequestDiscussion,
  requestDiscussionResource,
  resetRequestDiscussionCache,
} from './request-discussion-cache'
import { collectionFromPage } from './request-discussion-model'
import { deferred, discussion } from './request-discussion-test-fixtures'
import type { RequestDiscussionChanges, RequestDiscussionPage } from './request-discussion-types'

const page = (count: number): RequestDiscussionPage => ({
  discussions: Array.from({ length: count }, (_, index) => discussion(`discussion-${index}`, count - index)),
  next_cursor: 'older',
  snapshot_version: count,
})
const noChanges = async (after: number): Promise<RequestDiscussionChanges> => ({ discussions: [], through_position: after, has_more: false })

test('reopening retains more than 500 loaded discussions with their cursor intact', () => {
  resetRequestDiscussionCache()
  const session = openRequestDiscussion('request', page(1), noChanges)
  session.updateCollection(() => collectionFromPage(page(501)))
  const reopened = openRequestDiscussion('request', page(1), noChanges)
  assert.equal(reopened.collection.order.length, 501)
  assert.equal(reopened.collection.byId.size, 501)
  assert.equal(reopened.collection.nextCursor, 'older')
  assert.equal(reopened.sync, session.sync)
})

test('oversized active timelines render all loaded data and release it after leaving', () => {
  resetRequestDiscussionCache()
  const session = openRequestDiscussion('request', page(1), noChanges)
  const leave = requestDiscussionResource.subscribe('request', () => {})
  session.updateCollection(() => collectionFromPage(page(4001)))
  assert.equal(requestDiscussionResource.peek('request')?.collection.order.length, 4001)
  assert.equal(requestDiscussionResource.peek('request')?.collection.nextCursor, 'older')
  leave()
  assert.equal(requestDiscussionResource.peek('request'), null)
})

test('navigation reuses in-flight catch-up and pagination without losing completed data', async () => {
  resetRequestDiscussionCache()
  const changes = deferred<RequestDiscussionChanges>()
  let loads = 0
  const session = openRequestDiscussion('request', page(1), () => { loads++; return changes.promise })
  const catchUp = session.sync.catchUp()
  const older = deferred<RequestDiscussionPage>()
  const pagination = session.sync.paginate('older', () => older.promise)
  const reopened = openRequestDiscussion('request', page(1), noChanges)
  assert.equal(reopened.sync.catchUp(), catchUp)
  changes.resolve({ discussions: [discussion('new', 2)], through_position: 2, has_more: false })
  older.resolve({ discussions: [discussion('old', 0)], next_cursor: null, snapshot_version: 1 })
  await Promise.all([catchUp, pagination])
  const current = requestDiscussionResource.peek('request')!.collection
  assert.equal(loads, 1)
  assert.deepEqual(current.order, ['old', 'discussion-0', 'new'])
  assert.equal(current.nextCursor, null)
  assert.equal(current.snapshotVersion, 2)
})

test('reopening merges focused rows without resetting pagination and refreshes newer snapshots', async () => {
  resetRequestDiscussionCache()
  const session = openRequestDiscussion('request', page(1), noChanges)
  await session.sync.paginate('older', async () => ({ discussions: [discussion('old', 0)], next_cursor: null, snapshot_version: 1 }))
  await session.refresh({ ...page(1), discussions: [discussion('focused', 1)] })
  assert.deepEqual(requestDiscussionResource.peek('request')?.collection.order, ['old', 'discussion-0', 'focused'])
  assert.equal(requestDiscussionResource.peek('request')?.collection.nextCursor, null)
  await session.refresh({ discussions: [discussion('current', 2)], next_cursor: 'next', snapshot_version: 2 })
  assert.deepEqual(requestDiscussionResource.peek('request')?.collection.order, ['current'])
  assert.equal(requestDiscussionResource.peek('request')?.collection.nextCursor, 'next')
})

test('evicted catch-up stops draining and cannot write into a replacement session', async () => {
  resetRequestDiscussionCache()
  const pending = deferred<RequestDiscussionChanges>()
  let loads = 0
  const session = openRequestDiscussion('request', page(1), () => { loads++; return pending.promise })
  const catchingUp = session.sync.catchUp()
  resetRequestDiscussionCache()
  openRequestDiscussion('request', page(1), noChanges)
  pending.resolve({ discussions: [discussion('late', 2)], through_position: 2, has_more: true })
  await catchingUp
  assert.equal(loads, 1)
  assert.equal(requestDiscussionResource.peek('request')?.collection.byId.has('late'), false)
})
