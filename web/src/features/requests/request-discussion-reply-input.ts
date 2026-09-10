import type { CreateReplyInput } from './request-discussion-api'

export function requestDiscussionReplyBody(data: CreateReplyInput) {
  return {
    body_markdown: data.body_markdown,
    client_reply_id: data.client_reply_id,
    reply_to_reply_id: data.reply_to_reply_id,
    wait_after_reply: data.wait_after_reply,
  }
}
