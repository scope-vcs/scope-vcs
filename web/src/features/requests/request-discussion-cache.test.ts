import assert from 'node:assert/strict'
import test from 'node:test'
import {
  openRequestDiscussion,
  readRequestDiscussionScroll,
  requestDiscussionCacheKey,
  requestDiscussionResource,
  resetRequestDiscussionCache,
  writeRequestDiscussionScroll,
} from './request-discussion-cache'
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
  resetRequestDiscussionCache()
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
  resetRequestDiscussionCache()
  const original = openRequestDiscussion('request', page, loadChanges)
  for (let index = 0; index < 8; index++) openRequestDiscussion(`other-${index}`, page, loadChanges)
  assert.equal(requestDiscussionResource.peek('request'), null)
  original.setError('late error')
  assert.equal(requestDiscussionResource.peek('request'), null)
  const replacement = openRequestDiscussion('request', page, loadChanges)
  original.setLoadingMore(true)
  assert.equal(requestDiscussionResource.peek('request')?.loadingMore, false)
  resetRequestDiscussionCache()
  replacement.setError('late error')
  assert.equal(requestDiscussionResource.peek('request'), null)
})
