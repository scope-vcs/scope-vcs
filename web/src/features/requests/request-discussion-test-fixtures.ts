import type { RequestDiscussion, RequestDiscussionReplyView } from './request-discussion-types'

export function discussion(id: string, lastActivity: number, overrides: Partial<RequestDiscussion> = {}): RequestDiscussion {
  return {
    anchor: null,
    author: { handle: 'maya', id: 'user-maya' },
    body_markdown: `Discussion ${id}`,
    client_discussion_id: id,
    created_at_unix: lastActivity,
    id,
    last_activity_position: lastActivity,
    latest_replies: [],
    opened_position: lastActivity,
    read_through_position: lastActivity,
    reply_count: 0,
    request_id: 'request-1',
    resolved_at_unix: null,
    resolved_by: null,
    status: 'Open',
    unread_count: 0,
    ...overrides,
  }
}

export function reply(id: string, position: number, overrides: Partial<RequestDiscussionReplyView> = {}): RequestDiscussionReplyView {
  return {
    author: { handle: 'maya', id: 'user-maya' },
    body_markdown: `Reply ${id}`,
    created_at_unix: position,
    discussion_id: 'one',
    id,
    position,
    reply_to: null,
    ...overrides,
  }
}
