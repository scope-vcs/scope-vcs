import assert from 'node:assert/strict'
import test, { mock } from 'node:test'
import {
  openRequestDiscussion,
  readRequestDiscussionScroll,
  requestDiscussionCacheKey,
  requestDiscussionResource,
  writeRequestDiscussionScroll,
} from './request-discussion-cache'
import { resetViewerState } from '../../lib/viewer-state'
import { mergeDiscussion } from './request-discussion-model'
import { discussion } from './request-discussion-test-fixtures'

const page = { discussions: [discussion('one', 1)], next_cursor: 'older', snapshot_version: 1 }
const loadChanges = async () => ({ discussions: [], through_position: 1, has_more: false })

test('keys timeline views by viewer, repository access scope, and request', () => {
  const base = { repoId: 'scope/demo/member', requestId: 'request-1', viewerId: 'maya' }
  const key = requestDiscussionCacheKey(base)
  assert.equal(key, requestDiscussionCacheKey({ ...base }))
  for (const change of [{ requestId: 'request-2' }, { viewerId: 'ravi' }, { repoId: 'scope/demo/public' }]) {
    assert.notEqual(key, requestDiscussionCacheKey({ ...base, ...change }))
  }
})

test('reopening reuses the subscribed collection, expansion, scroll and pending pagination', () => {
  resetViewerState()
  const session = openRequestDiscussion('request', page, loadChanges)
  let notifications = 0
  const unsubscribe = requestDiscussionResource.subscribe('request', () => notifications++)
  session.updateCollection((current) => mergeDiscussion(current, { ...discussion('one', 1), expanded: true }), false)
  session.setLoadingMore(true)
  writeRequestDiscussionScroll('request', 240)
  unsubscribe()

  const reopened = openRequestDiscussion('request', { ...page }, loadChanges)
  assert.equal(reopened.collection.byId.get('one')?.expanded, true)
  assert.equal(reopened.dataGeneration, 0)
  assert.equal(reopened.loadingMore, true)
  assert.equal(readRequestDiscussionScroll('request'), 240)
  assert.equal(reopened.sync, session.sync)
  assert.equal(notifications, 3)
})

test('bounds retained views and ignores late writes after eviction or reset', () => {
  resetViewerState()
  const original = openRequestDiscussion('request', page, loadChanges)
  for (let index = 0; index < 8; index++) openRequestDiscussion(`other-${index}`, page, loadChanges)
  assert.equal(requestDiscussionResource.peek('request'), null)
  original.setError('late error')
  assert.equal(requestDiscussionResource.peek('request'), null)
  const replacement = openRequestDiscussion('request', page, loadChanges)
  original.setLoadingMore(true)
  assert.equal(requestDiscussionResource.peek('request')?.loadingMore, false)
  resetViewerState()
  replacement.setError('late error')
  assert.equal(requestDiscussionResource.peek('request'), null)
})

test('a pushed revision refreshes loaded line anchors in place without dropping other discussions', async () => {
  resetViewerState()
  const anchor = {
    revision_id: 'revision-1', revision_position: 1, commit_oid: null, path: '/src/lib.rs',
    lines: { side: 'new', start_line: 3, end_line: 4 }, latest: null,
  } as const
  const lined = discussion('lined', 2, { anchor })
  const session = openRequestDiscussion('request', { discussions: [lined, discussion('plain', 1)], next_cursor: null, snapshot_version: 2 }, loadChanges)
  const moved = { ...anchor, latest: { kind: 'mapped', revision_id: 'revision-2', start_line: 5, end_line: 6 } } as const
  const load = mock.fn(async (limit: number) => ({
    discussions: [discussion('lined', 2, { anchor: moved })].slice(0, limit),
    next_cursor: null,
    snapshot_version: 3,
  }))

  await session.refreshAnchors(load)

  assert.equal(load.mock.callCount(), 1)
  assert.equal(load.mock.calls[0].arguments[0], 2)
  const current = requestDiscussionResource.peek('request')?.collection
  assert.deepEqual(current?.byId.get('lined')?.anchor?.latest, moved.latest)
  assert.ok(current?.byId.get('plain'))

  resetViewerState()
  const unanchored = openRequestDiscussion('request', page, loadChanges)
  const unused = mock.fn(load)
  await unanchored.refreshAnchors(unused)
  assert.equal(unused.mock.callCount(), 0)
})
