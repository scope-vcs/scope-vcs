import type { RequestParams } from '@/api/types'
import { loadRequestDiscussionSession, requestDiscussionCacheKey } from '@/features/requests/request-discussion-cache'
import { loadRequestDiscussionPage, loadRequestDiscussionChanges } from './-request-discussion-actions'

export function loadRequestDiscussionRoutePage(data: RequestParams, scope: string, viewerId: string | null, discussionId?: string) {
  const page = loadRequestDiscussionSession({
    key: requestDiscussionCacheKey({ repoId: scope, requestId: data.request_id, viewerId: viewerId ?? 'anonymous' }),
    focusedDiscussionId: discussionId,
    load: () => loadRequestDiscussionPage({ data: { ...data, discussion_id: discussionId } }),
    loadChanges: (after) => loadRequestDiscussionChanges({ data: { ...data, after } }),
  })
  return page instanceof Promise ? page.catch(() => null) : page
}
