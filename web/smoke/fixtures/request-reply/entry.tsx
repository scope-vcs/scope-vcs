import type { RepoLiveState } from '@/api/types'
import { RepoLayoutProvider } from '@/features/repo-detail/repo-layout-context'
import { RequestAttachmentProvider } from '@/features/requests/request-attachment-context'
import type { CreateReplyInput } from '@/features/requests/request-discussion-api'
import { RequestDiscussionThread } from '@/features/requests/request-discussion-thread'
import type { RequestDiscussionReply, RequestDiscussionView } from '@/features/requests/request-discussion-types'
import { useState } from 'react'
import { createRoot } from 'react-dom/client'

const calls: CreateReplyInput[] = []
Object.assign(window, { calls })

const params = { owner: 'dev', repo: 'demo', request_id: 'request' }
const live = { repo: { id: 'repo', owner_handle: 'dev', name: 'demo', access: { actor: 'Owner' } } } as RepoLiveState
const viewer = { handle: 'viewer', id: 'viewer' }
const reply = (id: string, handle: string, body: string, position: number): RequestDiscussionReply => ({
  author: { handle, id: handle } as RequestDiscussionReply['author'],
  body_markdown: body,
  created_at_unix: 1,
  discussion_id: 'discussion',
  id,
  position,
  reply_to: null,
})
const replies = [
  reply('reply-alice', 'alice', 'Cap retries at five.', 2),
  reply('reply-bob', 'bob', 'Cap retries at three.', 3),
]
const discussion = {
  anchor: null,
  author: viewer,
  body_markdown: 'Which retry cap?',
  client_discussion_id: 'discussion',
  created_at_unix: 1,
  expanded: true,
  id: 'discussion',
  last_activity_position: 3,
  latest_replies: replies,
  opened_position: 1,
  read_through_position: 3,
  reply_count: replies.length,
  request_id: 'request',
  resolved_at_unix: null,
  resolved_by: null,
  status: 'Open',
  unread_count: 0,
} as RequestDiscussionView

async function createReply(input: CreateReplyInput) {
  calls.push(input)
  if (calls.length === 1) throw new Error('Service unavailable')
  const quoted = replies.find(({ id }) => id === input.reply_to_reply_id)
  return {
    discussion: { ...discussion, reply_count: replies.length + 1 },
    reply: {
      ...reply(input.client_reply_id, viewer.handle, input.body_markdown, 4),
      reply_to: quoted ? { author: quoted.author, body_markdown: quoted.body_markdown, id: quoted.id, position: quoted.position } : null,
    },
  }
}
const actions = {
  createReply,
  loadReplies: async () => ({ next_before_position: null, replies: [] }),
  reopenAndReply: createReply,
}
const attachmentActions = {
  list: async () => ({ attachments: [] }),
  limits: async () => ({ accepted_photo_media_types: [], accepted_video_media_types: [], max_attachments_per_content: 10 }),
}
const noop = () => {}

function App() {
  const [composerOpen, setComposerOpen] = useState(false)
  return (
    <RepoLayoutProvider live={live} subscribe={() => noop}>
      <RequestAttachmentProvider actions={attachmentActions as never} live={live} requestId="request" viewerId={viewer.id}>
        <RequestDiscussionThread
          actions={actions}
          actor={viewer}
          canReply
          canResolve
          canWaitAfterReply={false}
          composerOpen={composerOpen}
          discussion={discussion}
          onCloseComposer={() => setComposerOpen(false)}
          onExpandedChange={noop}
          onMarkRead={async () => {}}
          onOpenComposer={() => setComposerOpen(true)}
          onPatch={noop}
          onResolve={async () => {}}
          onRetryRoot={async () => false}
          params={params}
        />
      </RequestAttachmentProvider>
    </RepoLayoutProvider>
  )
}

createRoot(document.getElementById('root')!).render(<App />)
