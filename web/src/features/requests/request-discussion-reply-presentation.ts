import { unixCalendarDay } from '../../lib/date-format'
import type { RequestDiscussionReplyView } from './request-discussion-types'
import { isSameActor } from './request-actor'

export function replyFragment(discussionId: string, replyId: string) {
  const fragment = new URLSearchParams({
    discussion: discussionId,
    reply: replyId,
  })
  return `#${fragment.toString()}`
}

export function replyTargetFromFragment(hash: string) {
  if (!hash.startsWith('#')) return null
  const fragment = new URLSearchParams(hash.slice(1))
  const discussionId = fragment.get('discussion')
  const replyId = fragment.get('reply')
  if (!discussionId || !replyId) return null
  return { discussionId, replyId }
}

const GROUP_WINDOW_SECONDS = 5 * 60

export function shouldGroupReplies(
  previous: RequestDiscussionReplyView | null,
  current: RequestDiscussionReplyView,
  boundary: { date: boolean; unread: boolean },
) {
  return Boolean(
    previous &&
    !boundary.date &&
    !boundary.unread &&
    !previous.pending &&
    !current.pending &&
    isSameActor(previous.author, current.author) &&
    current.created_at_unix >= previous.created_at_unix &&
    current.created_at_unix - previous.created_at_unix <=
      GROUP_WINDOW_SECONDS,
  )
}

// Day boundaries follow the same zone as the label above them: UTC through
// hydration, then the viewer's own calendar.
export function sameCalendarDate(
  leftUnix: number,
  rightUnix: number,
  hydrated: boolean,
) {
  return unixCalendarDay(leftUnix, hydrated) === unixCalendarDay(rightUnix, hydrated)
}
