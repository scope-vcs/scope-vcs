import type { RequestParams } from '@/api/types'
import { loadRequestDiscussion, requestDiscussionCacheKey } from '@/features/requests/discussion/request-discussion-cache'
import type { RequestDiscussionPage } from '@/features/requests/discussion/request-discussion-types'
import { loadRequestDiscussionPage } from './-request-discussion-actions'

export function loadRequestDiscussionFirstPage(data: RequestParams, discussionId?: string) {
  return loadRequestDiscussionPage({ data: { ...data, discussion_id: discussionId } })
}

export function loadRequestDiscussionRoute(
  data: RequestParams,
  scope: string,
  viewerId: string | null,
  discussionId?: string,
  started?: Promise<RequestDiscussionPage | null>,
) {
  let pending = started
  const available = loadRequestDiscussion({
    key: requestDiscussionCacheKey({ repoId: scope, requestId: data.request_id, viewerId: viewerId ?? 'anonymous' }),
    focusedDiscussionId: discussionId,
    load: () => {
      const page = pending ?? loadRequestDiscussionFirstPage(data, discussionId)
      pending = undefined
      return page
    },
  })
  return available === true ? true : available.catch(() => false)
}
