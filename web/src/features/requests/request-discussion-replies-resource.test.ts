import assert from 'node:assert/strict'
import test from 'node:test'
import { openRequestDiscussionReplies, requestDiscussionRepliesResource } from './request-discussion-replies-resource'
import { acknowledgeReply, beforePositionForNextReplyPage, insertOptimisticReply, markReplyFailed, mergeReplyPage, mergeReplyTarget } from './request-discussion-replies-model'
import { reply } from './request-discussion-test-fixtures'

test('reopening preserves loaded pages and refreshes the newest page when the preview advances', () => {
  requestDiscussionRepliesResource.clear()
  const session = openRequestDiscussionReplies('viewer/access/request/thread')
  session.update((current) => mergeReplyPage(current, { replies: [reply('older', 1), reply('latest', 2)], next_before_position: 1 }, [], true))
  const reopened = openRequestDiscussionReplies('viewer/access/request/thread')
  assert.equal(reopened.update, session.update)
  assert.equal(beforePositionForNextReplyPage(reopened, [reply('latest', 2)]), 1)
  assert.equal(beforePositionForNextReplyPage(reopened, [reply('new', 3)]), undefined)
  assert.deepEqual(reopened.replies.map(({ id }) => id), ['older', 'latest'])
  assert.equal(openRequestDiscussionReplies('other-viewer/access/request/thread').replies.length, 0)
  assert.equal(openRequestDiscussionReplies('viewer/other-access/request/thread').replies.length, 0)
})

test('optimistic failure, retry and acknowledgment update the persistent reply owner', () => {
  requestDiscussionRepliesResource.clear()
  const session = openRequestDiscussionReplies('request')
  session.update((current) => insertOptimisticReply(current, reply('client', Number.MAX_SAFE_INTEGER, { pending: 'sending' })))
  session.update((current) => markReplyFailed(current, 'client'))
  const reopened = openRequestDiscussionReplies('request')
  assert.equal(reopened.replies[0]?.pending, 'failed')
  reopened.update((current) => insertOptimisticReply(current, reply('client', Number.MAX_SAFE_INTEGER, { pending: 'sending' })))
  session.update((current) => acknowledgeReply(current, 'client', reply('server', 4)))
  assert.deepEqual(openRequestDiscussionReplies('request').replies.map(({ id, pending }) => ({ id, pending })), [{ id: 'server', pending: undefined }])
})

test('target operations survive reopening without changing the older-page cursor', () => {
  requestDiscussionRepliesResource.clear()
  const session = openRequestDiscussionReplies('request')
  session.update((current) => mergeReplyPage(current, { replies: [reply('new', 10)], next_before_position: 10 }, [], true))
  const target = { replyId: 'target', promise: Promise.resolve(true) }
  session.update((current) => ({ ...current, target }))
  assert.equal(openRequestDiscussionReplies('request').target, target)
  session.update((current) => mergeReplyTarget(current, { replies: [reply('target', 2)], next_before_position: 2 }))
  assert.equal(openRequestDiscussionReplies('request').page.nextBeforePosition, 10)
  assert.equal(openRequestDiscussionReplies('request').target, target)
})

test('cleared sessions cannot write into a replacement thread', () => {
  requestDiscussionRepliesResource.clear()
  const old = openRequestDiscussionReplies('request')
  requestDiscussionRepliesResource.clear()
  openRequestDiscussionReplies('request')
  old.update((current) => insertOptimisticReply(current, reply('late', 1)))
  assert.equal(openRequestDiscussionReplies('request').replies.length, 0)
})
