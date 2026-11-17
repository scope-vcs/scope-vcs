import type { RepoLiveState } from '@/api/types'
import type { RequestSummaryResponse } from '@/api/types.generated'
import { RepoLayoutProvider } from '@/features/repo-detail/repo-layout-context'
import { RequestAttachmentProvider } from '@/features/requests/request-attachment-context'
import {
  loadRequestDiscussion,
  requestDiscussionCacheKey,
  requestDiscussionResource,
  requestDiscussionState,
} from '@/features/requests/request-discussion-cache'
import type { RequestDiscussionPage, RequestDiscussionView } from '@/features/requests/request-discussion-types'
import { RequestDiscussionWorkbench } from '@/features/requests/request-discussion-workbench'
import { useState } from 'react'
import { createRoot } from 'react-dom/client'

const RETAINED_TIMELINES = 8
const params = { owner: 'dev', repo: 'demo', request_id: 'request' }
const viewer = { kind: 'user' as const, handle: 'viewer', id: 'viewer' }
const live = { repo: { id: 'repo', owner_handle: 'dev', name: 'demo', views: [{ id: 'public', name: 'Public', includes: [], readers: 'anyone' as const }], access: { actor: 'Owner' } } } as RepoLiveState
const discussion = {
  anchor: null,
  author: viewer,
  body_markdown: 'Which retry cap?',
  client_discussion_id: 'discussion',
  created_at_unix: 1,
  id: 'discussion',
  last_activity_position: 1,
  latest_replies: [],
  opened_position: 1,
  read_through_position: 1,
  reply_count: 0,
  request_id: 'request',
  resolved_at_unix: null,
  resolved_by: null,
  status: 'Open',
  unread_count: 0,
} as RequestDiscussionView
const page: RequestDiscussionPage = { discussions: [discussion], next_cursor: 'older', snapshot_version: 1 }
const olderPage: RequestDiscussionPage = {
  discussions: [{ ...discussion, body_markdown: 'An older question', client_discussion_id: 'older', id: 'older', last_activity_position: 0, opened_position: 0 }],
  next_cursor: null,
  snapshot_version: 1,
}
const server = { olderLoads: 0, pageLoads: 0 }
const heldOlderLoads: Array<() => void> = []
const loadPage = async (input?: { cursor?: string }) => {
  if (!input?.cursor) {
    server.pageLoads += 1
    return page
  }
  server.olderLoads += 1
  await new Promise<void>((resolve) => heldOlderLoads.push(resolve))
  return olderPage
}
const unused = async () => { throw new Error('This fixture does not exercise this action.') }
const actions = {
  create: unused,
  load: loadPage,
  loadChanges: async ({ after }: { after: number }) => ({ discussions: [], through_position: after, has_more: false }),
  markRead: async () => {},
  resolve: unused,
}
const threadActions = {
  createReply: unused,
  loadReplies: async () => ({ next_before_position: null, replies: [] }),
  reopenAndReply: unused,
}
const attachmentActions = {
  list: async () => ({ attachments: [] }),
  limits: async () => ({ accepted_photo_media_types: [], accepted_video_media_types: [], max_attachments_per_content: 10 }),
}
const request = { id: 'request', head_oid: 'head', state: 'Open', author_user_id: viewer.id } as RequestSummaryResponse
const scope = 'scope'
const key = requestDiscussionCacheKey({ repoId: scope, requestId: params.request_id, viewerId: viewer.id })

Object.assign(window, { fixture: { server, finishOlderLoad: () => heldOlderLoads.shift()?.() } })

await loadRequestDiscussion({ key, load: () => loadPage() })
for (let index = 0; index < RETAINED_TIMELINES; index++) {
  requestDiscussionResource.write(`other-${index}`, requestDiscussionState(page))
}

function App() {
  const [shown, setShown] = useState(true)
  return (
    <RepoLayoutProvider live={live} subscribe={() => () => {}} summaryIdentity="fixture">
      <RequestAttachmentProvider actions={attachmentActions as never} live={live} requestId="request" viewerId={viewer.id}>
        <output aria-label="Retained before the view subscribes">{retainedBeforeMount}</output>
        <button onClick={() => setShown((value) => !value)} type="button">Toggle discussion</button>
        {shown ? (
          <RequestDiscussionWorkbench
            actions={actions}
            actor={viewer}
            canResolve={() => false}
            params={params}
            pending={<p role="status">Loading request discussion</p>}
            permissions={{ canOpenDiscussion: true, canReply: true, canWaitAfterReply: false }}
            repoId={scope}
            request={request}
            threadActions={threadActions}
            unavailable={<p>Discussion is unavailable</p>}
            viewerId={viewer.id}
          />
        ) : null}
      </RequestAttachmentProvider>
    </RepoLayoutProvider>
  )
}

const retainedBeforeMount = String(requestDiscussionResource.peek(key) !== null)
createRoot(document.getElementById('root')!).render(<App />)
