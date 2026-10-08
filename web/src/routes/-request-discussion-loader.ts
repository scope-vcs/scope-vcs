import type { RequestParams } from '@/api/types'
import { loadRequestDiscussionSession, requestDiscussionCacheKey } from '@/features/requests/request-discussion-cache'
import { loadRequestDiscussionPage, loadRequestDiscussionChanges } from './-request-discussion-actions'

export function loadRequestDiscussionRoutePage(data: RequestParams, scope: string | null, viewerId: string | null, discussionId?: string) {
  const load = () => loadRequestDiscussionPage({ data: { ...data, discussion_id: discussionId } })
  const page = scope
    ? loadRequestDiscussionSession({
        key: requestDiscussionCacheKey({ repoId: scope, requestId: data.request_id, viewerId: viewerId ?? 'anonymous' }),
        focusedDiscussionId: discussionId,
        load,
        loadChanges: (after) => loadRequestDiscussionChanges({ data: { ...data, after } }),
      })
    : load()
  return page instanceof Promise ? page.catch(() => null) : page
}
