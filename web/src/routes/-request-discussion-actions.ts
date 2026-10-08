import { loadOptionalResource } from '@/api/http'
import { parseLoadDiscussionsInput, parseLoadDiscussionChangesInput } from '@/api/request-inputs'
import { loadRequestDiscussionChangesForRequest, loadRequestDiscussionsForRequest } from '@/features/requests/request-discussion-api'
import { includeFocusedDiscussion } from '@/features/requests/request-discussion-model'
import { createServerFn } from '@tanstack/react-start'

export const loadRequestDiscussionPage = createServerFn({ method: 'GET' })
  .validator(parseLoadDiscussionsInput)
  .handler(async ({ data }) => {
    const [page, focused] = await Promise.all([
      loadOptionalResource(() => loadRequestDiscussionsForRequest({
        owner: data.owner,
        repo: data.repo,
        request_id: data.request_id,
      })),
      data.discussion_id
        ? loadOptionalResource(() => loadRequestDiscussionsForRequest(data))
        : Promise.resolve(null),
    ])
    return includeFocusedDiscussion(page, focused)
  })

export const loadRequestDiscussionChanges = createServerFn({ method: 'GET' })
  .validator(parseLoadDiscussionChangesInput)
  .handler(({ data }) => loadRequestDiscussionChangesForRequest(data))
