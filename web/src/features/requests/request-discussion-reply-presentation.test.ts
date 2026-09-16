import assert from 'node:assert/strict'
import test from 'node:test'
import { reply as replyFixture } from './request-discussion-test-fixtures'
import {
  replyFragment,
  replyTargetFromFragment,
  sameCalendarDate,
  shouldGroupReplies,
} from './request-discussion-reply-presentation'
import type { RequestDiscussionReplyView } from './request-discussion-types'

test('groups consecutive messages from one author for five minutes', () => {
  const previous = reply(100)

  assert.equal(shouldGroupReplies(previous, reply(400), noBoundary), true)
  assert.equal(shouldGroupReplies(previous, reply(401), noBoundary), false)
  assert.equal(
    shouldGroupReplies(previous, { ...reply(200), author: otherAuthor }, noBoundary),
    false,
  )
})

test('date, unread, and pending states start a fresh message group', () => {
  const previous = reply(100)

  assert.equal(shouldGroupReplies(previous, reply(101), { date: true, unread: false }), false)
  assert.equal(shouldGroupReplies(previous, reply(101), { date: false, unread: true }), false)
  assert.equal(
    shouldGroupReplies(previous, { ...reply(101), pending: 'sending' }, noBoundary),
    false,
  )
})

test('date boundaries use a deterministic UTC calendar day before hydration', () => {
  assert.equal(sameCalendarDate(86_399, 86_400, false), false)
  assert.equal(sameCalendarDate(86_400, 86_401, false), true)
})

test('a hydrated viewer gets boundaries on their own calendar day', () => {
  const previousZone = process.env.TZ
  process.env.TZ = 'America/New_York'
  try {
    // 1970-01-01T04:00Z and 1970-01-01T05:00Z share a UTC day but straddle
    // midnight in New York.
    assert.equal(sameCalendarDate(14_400, 18_000, true), false)
    assert.equal(sameCalendarDate(14_400, 18_000, false), true)
  } finally {
    process.env.TZ = previousZone
  }
})

test('reply fragments identify one discussion and reply', () => {
  const fragment = replyFragment('discussion/a', 'reply #1')

  assert.equal(
    fragment,
    '#discussion=discussion%2Fa&reply=reply+%231',
  )
  assert.deepEqual(replyTargetFromFragment(fragment), {
    discussionId: 'discussion/a',
    replyId: 'reply #1',
  })
  assert.equal(replyTargetFromFragment('#discussion=one'), null)
  assert.equal(replyTargetFromFragment('discussion=one&reply=two'), null)
})

const noBoundary = { date: false, unread: false }
const otherAuthor = { handle: 'ravi', id: 'user-ravi' }

function reply(createdAtUnix: number): RequestDiscussionReplyView {
  return replyFixture(`reply-${createdAtUnix}`, createdAtUnix, {
    body_markdown: 'Reply',
    discussion_id: 'discussion',
  })
}
