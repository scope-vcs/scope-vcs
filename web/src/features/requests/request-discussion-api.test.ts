import assert from 'node:assert/strict'
import test from 'node:test'
import { requestDiscussionReplyBody } from './request-discussion-reply-input'

test('reply request body keeps ordinary and wait submissions distinct', () => {
  const input = {
    owner: 'scope',
    repo: 'vcs',
    request_id: 'request_1',
    discussion_id: 'discussion_1',
    body_markdown: 'Please take another look.',
    client_reply_id: 'reply_1',
    reply_to_reply_id: null,
  }

  assert.deepEqual(
    requestDiscussionReplyBody({ ...input, wait_after_reply: false }),
    {
      body_markdown: input.body_markdown,
      client_reply_id: input.client_reply_id,
      reply_to_reply_id: null,
      wait_after_reply: false,
    },
  )
  assert.equal(
    requestDiscussionReplyBody({ ...input, wait_after_reply: true })
      .wait_after_reply,
    true,
  )
})
