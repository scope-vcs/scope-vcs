import type { RequestParams } from '@/api/types'
import { loadRequestDiscussion, requestDiscussionCacheKey } from '@/features/requests/request-discussion-cache'
import { loadRequestDiscussionPage } from './-request-discussion-actions'

export function loadRequestDiscussionRoute(data: RequestParams, scope: string, viewerId: string | null, discussionId?: string) {
  const available = loadRequestDiscussion({
    key: requestDiscussionCacheKey({ repoId: scope, requestId: data.request_id, viewerId: viewerId ?? 'anonymous' }),
    focusedDiscussionId: discussionId,
    load: () => loadRequestDiscussionPage({ data: { ...data, discussion_id: discussionId } }),
  })
  return available === true ? true : available.catch(() => false)
}
