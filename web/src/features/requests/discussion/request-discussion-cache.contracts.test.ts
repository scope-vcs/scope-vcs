import assert from 'node:assert/strict'
import test, { mock } from 'node:test'
import {
  loadRequestDiscussion,
  requestDiscussionResource,
  requestDiscussionState,
  updateRequestDiscussionCollection,
} from './request-discussion-cache'
import { resetViewerState } from '../../../lib/viewer-state'
import { appendDiscussionPage, collectionFromPage } from './request-discussion-model'
import { deferred, discussion } from './request-discussion-test-fixtures'
import type { RequestDiscussionPage } from './request-discussion-types'

const page = (count: number): RequestDiscussionPage => ({
  discussions: Array.from({ length: count }, (_, index) => discussion(`discussion-${index}`, count - index)),
  next_cursor: 'older',
  snapshot_version: count,
})
const olderPage = { discussions: [discussion('old', 0)], next_cursor: null, snapshot_version: 1 }

function retain(key: string, loaded: RequestDiscussionPage) {
  const state = requestDiscussionState(loaded)
  requestDiscussionResource.write(key, state)
  return state
}

function retainWithOlderPage() {
  const state = retain('request', page(1))
  updateRequestDiscussionCollection('request', state.epoch, (current) => appendDiscussionPage(current, olderPage))
  return state
}

const retainedCollection = () => requestDiscussionResource.peek('request')?.collection

test('reopening retains more than 500 loaded discussions with their cursor intact', () => {
  resetViewerState()
  const state = retain('request', page(1))
  updateRequestDiscussionCollection('request', state.epoch, () => collectionFromPage(page(501)))
  assert.equal(loadRequestDiscussion({ key: 'request', load: async () => page(1) }), true)
  assert.equal(retainedCollection()?.order.length, 501)
  assert.equal(retainedCollection()?.byId.size, 501)
  assert.equal(retainedCollection()?.nextCursor, 'older')
})

test('oversized active timelines render all loaded data and release it after leaving', () => {
  resetViewerState()
  const state = retain('request', page(1))
  const leave = requestDiscussionResource.subscribe('request', () => {})
  updateRequestDiscussionCollection('request', state.epoch, () => collectionFromPage(page(4001)))
  assert.equal(retainedCollection()?.order.length, 4001)
  assert.equal(retainedCollection()?.nextCursor, 'older')
  leave()
  assert.equal(requestDiscussionResource.peek('request'), null)
})

test('a write issued for a replaced timeline cannot reach its replacement', () => {
  resetViewerState()
  const replaced = retain('request', page(1))
  resetViewerState()
  retain('request', page(1))
  updateRequestDiscussionCollection('request', replaced.epoch, () => collectionFromPage({
    ...page(1), discussions: [discussion('late', 2)],
  }))
  assert.equal(retainedCollection()?.byId.has('late'), false)
})

test('concurrent navigation loads the initial discussion page once and reopening preserves pagination', async () => {
  resetViewerState()
  const response = deferred<RequestDiscussionPage>()
  const load = mock.fn(() => response.promise)
  const first = loadRequestDiscussion({ key: 'request', load })
  const second = loadRequestDiscussion({ key: 'request', load })
  response.resolve(page(1))
  await Promise.all([first, second])
  const state = requestDiscussionResource.peek('request')
  assert.ok(state)
  updateRequestDiscussionCollection('request', state.epoch, (current) => appendDiscussionPage(current, olderPage))
  assert.equal(await loadRequestDiscussion({ key: 'request', load }), true)
  assert.equal(load.mock.callCount(), 1)
  assert.equal(retainedCollection()?.nextCursor, null)
  assert.deepEqual(retainedCollection()?.order, ['old', 'discussion-0'])
})

test('focused navigation that joins an initial load still fetches its missing discussion', async () => {
  resetViewerState()
  const initial = deferred<RequestDiscussionPage>()
  const focusedLoad = mock.fn(async () => ({ ...page(1), discussions: [discussion('focused', 1)] }))
  const first = loadRequestDiscussion({ key: 'request', load: () => initial.promise })
  const focused = loadRequestDiscussion({ key: 'request', focusedDiscussionId: 'focused', load: focusedLoad })
  initial.resolve(page(1))
  await first
  await focused
  assert.equal(retainedCollection()?.order.includes('focused'), true)
  assert.equal(focusedLoad.mock.callCount(), 1)
})

test('focused navigation merges missing rows into the retained timeline without resetting pagination', async () => {
  resetViewerState()
  retainWithOlderPage()
  await loadRequestDiscussion({
    key: 'request', focusedDiscussionId: 'focused',
    load: async () => ({ ...page(1), discussions: [discussion('focused', 1)] }),
  })
  assert.deepEqual(retainedCollection()?.order, ['old', 'discussion-0', 'focused'])
  assert.equal(retainedCollection()?.nextCursor, null)
})

for (const [failure, load] of [
  ['unavailable', async () => null],
  ['offline', async () => { throw new Error('offline') }],
] as const) {
  test(`a failed focused load retains loaded discussions and pagination: ${failure}`, async () => {
    resetViewerState()
    retainWithOlderPage()
    assert.equal(await loadRequestDiscussion({ key: 'request', focusedDiscussionId: 'missing', load }), true)
    assert.deepEqual(retainedCollection()?.order, ['old', 'discussion-0'])
    assert.equal(retainedCollection()?.nextCursor, null)
  })
}

test('a focused response cannot return a prior viewer timeline after reset', async () => {
  resetViewerState()
  retain('request', page(1))
  const response = deferred<RequestDiscussionPage>()
  const loading = loadRequestDiscussion({ key: 'request', focusedDiscussionId: 'missing', load: () => response.promise })
  resetViewerState()
  response.resolve({ ...page(1), discussions: [discussion('missing', 1)] })
  await assert.rejects(async () => loading, /Resource is no longer available/)
  assert.equal(requestDiscussionResource.peek('request'), null)
})

test('a discussion first-page response cannot publish after the viewer changes', async () => {
  resetViewerState()
  const response = deferred<RequestDiscussionPage>()
  const loading = loadRequestDiscussion({ key: 'request', load: () => response.promise })
  await Promise.resolve()
  resetViewerState()
  response.resolve(page(1))
  await assert.rejects(async () => loading, /Resource is no longer available/)
  assert.equal(requestDiscussionResource.peek('request'), null)
})
