import assert from 'node:assert/strict'
import test, { mock } from 'node:test'
import {
  openRequestDiscussion,
  loadRequestDiscussionSession,
  requestDiscussionResource,
} from './request-discussion-cache'
import { resetViewerState } from '../../lib/viewer-state'
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
  resetViewerState()
  const session = openRequestDiscussion('request', page(1), noChanges)
  session.updateCollection(() => collectionFromPage(page(501)))
  const reopened = openRequestDiscussion('request', page(1), noChanges)
  assert.equal(reopened.collection.order.length, 501)
  assert.equal(reopened.collection.byId.size, 501)
  assert.equal(reopened.collection.nextCursor, 'older')
  assert.equal(reopened.sync, session.sync)
})

test('oversized active timelines render all loaded data and release it after leaving', () => {
  resetViewerState()
  const session = openRequestDiscussion('request', page(1), noChanges)
  const leave = requestDiscussionResource.subscribe('request', () => {})
  session.updateCollection(() => collectionFromPage(page(4001)))
  assert.equal(requestDiscussionResource.peek('request')?.collection.order.length, 4001)
  assert.equal(requestDiscussionResource.peek('request')?.collection.nextCursor, 'older')
  leave()
  assert.equal(requestDiscussionResource.peek('request'), null)
})

test('navigation reuses in-flight catch-up and pagination without losing completed data', async () => {
  resetViewerState()
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
  resetViewerState()
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
  resetViewerState()
  const pending = deferred<RequestDiscussionChanges>()
  let loads = 0
  const session = openRequestDiscussion('request', page(1), () => { loads++; return pending.promise })
  const catchingUp = session.sync.catchUp()
  resetViewerState()
  openRequestDiscussion('request', page(1), noChanges)
  pending.resolve({ discussions: [discussion('late', 2)], through_position: 2, has_more: true })
  await catchingUp
  assert.equal(loads, 1)
  assert.equal(requestDiscussionResource.peek('request')?.collection.byId.has('late'), false)
})


test('concurrent navigation loads the initial discussion page once and reopening preserves pagination', async () => {
  resetViewerState()
  const response = deferred<RequestDiscussionPage>()
  const load = mock.fn(() => response.promise)
  const options = { key: 'request', load, loadChanges: noChanges }
  const first = loadRequestDiscussionSession(options)
  const second = loadRequestDiscussionSession(options)
  response.resolve(page(1))
  await Promise.all([first, second])
  const session = requestDiscussionResource.peek('request')!
  await session.sync.paginate('older', async () => ({ discussions: [discussion('old', 0)], next_cursor: null, snapshot_version: 1 }))
  const reopened = await loadRequestDiscussionSession(options)
  assert.equal(load.mock.callCount(), 1)
  assert.equal(reopened?.next_cursor, null)
  assert.deepEqual(reopened?.discussions.map(({ id }) => id), ['discussion-0', 'old'])
})

test('focused navigation that joins an initial load still fetches its missing discussion', async () => {
  resetViewerState()
  const initial = deferred<RequestDiscussionPage>()
  const focusedLoad = mock.fn(async () => ({ ...page(1), discussions: [discussion('focused', 1)] }))
  const first = loadRequestDiscussionSession({ key: 'request', load: () => initial.promise, loadChanges: noChanges })
  const focused = loadRequestDiscussionSession({ key: 'request', focusedDiscussionId: 'focused', load: focusedLoad, loadChanges: noChanges })
  initial.resolve(page(1))
  await first
  const loaded = await focused
  assert.equal(loaded?.discussions.some(({ id }) => id === 'focused'), true)
  assert.equal(focusedLoad.mock.callCount(), 1)
})

test('focused navigation merges missing rows through the retained owner', async () => {
  resetViewerState()
  const catchUp = mock.fn(noChanges)
  const session = openRequestDiscussion('request', page(1), catchUp)
  await session.sync.paginate('older', async () => ({ discussions: [discussion('old', 0)], next_cursor: null, snapshot_version: 1 }))
  const focused = await loadRequestDiscussionSession({
    key: 'request', focusedDiscussionId: 'focused',
    load: async () => ({ ...page(1), discussions: [discussion('focused', 1)] }), loadChanges: noChanges,
  })
  assert.equal(requestDiscussionResource.peek('request')?.collection.byId.has('focused'), true)
  assert.equal(focused?.next_cursor, null)
  assert.equal(catchUp.mock.callCount(), 0)
})

for (const [failure, load] of [
  ['unavailable', async () => null],
  ['offline', async () => { throw new Error('offline') }],
] as const) {
  test(`a failed focused load retains loaded discussions and pagination: ${failure}`, async () => {
    resetViewerState()
    const session = openRequestDiscussion('request', page(1), noChanges)
    await session.sync.paginate('older', async () => ({ discussions: [discussion('old', 0)], next_cursor: null, snapshot_version: 1 }))
    const retained = await loadRequestDiscussionSession({ key: 'request', focusedDiscussionId: 'missing', load, loadChanges: noChanges })
    assert.deepEqual(retained?.discussions.map(({ id }) => id), ['discussion-0', 'old'])
    assert.equal(retained?.next_cursor, null)
  })
}

test('a focused response cannot return a prior viewer timeline after reset', async () => {
  resetViewerState()
  openRequestDiscussion('request', page(1), noChanges)
  const response = deferred<RequestDiscussionPage>()
  const loading = loadRequestDiscussionSession({ key: 'request', focusedDiscussionId: 'missing', load: () => response.promise, loadChanges: noChanges })
  resetViewerState()
  response.resolve({ ...page(1), discussions: [discussion('missing', 1)] })
  await assert.rejects(async () => loading, /Resource is no longer available/)
  assert.equal(requestDiscussionResource.peek('request'), null)
})

test('a discussion first-page response cannot publish after the viewer changes', async () => {
  resetViewerState()
  const response = deferred<RequestDiscussionPage>()
  const loading = loadRequestDiscussionSession({ key: 'request', load: () => response.promise, loadChanges: noChanges })
  await Promise.resolve()
  resetViewerState()
  response.resolve(page(1))
  await assert.rejects(async () => loading, /Resource is no longer available/)
  assert.equal(requestDiscussionResource.peek('request'), null)
})
