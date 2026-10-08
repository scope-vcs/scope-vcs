import {
  parseLoadDiscussionsInput,
  parseLoadRepliesInput,
  parseCreateDiscussionInput,
  parseCreateReplyInput,
  parseDiscussionActionInput,
  parseMarkDiscussionReadInput,
} from '@/api/request-inputs'
import {
  createRequestDiscussionForRequest,
  createRequestDiscussionReplyForRequest,
  loadRequestDiscussionRepliesForRequest,
  loadRequestDiscussionsForRequest,
  markRequestDiscussionReadForRequest,
  reopenAndReplyToRequestDiscussionForRequest,
  resolveRequestDiscussionForRequest,
} from '@/features/requests/request-discussion-api'
import type { RequestDiscussionPage } from '@/features/requests/request-discussion-types'
import { RequestDiscussionView } from '@/features/requests/request-discussion-view'
import { RequestDiscussionPending } from '@/features/requests/request-page-pending'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { Await, createFileRoute, getRouteApi } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { Suspense } from 'react'
import { useRequestState } from '@/features/requests/request-state-context'
import { loadRequestDiscussionChanges } from '@/routes/-request-discussion-actions'
import { MessageSquare } from 'lucide-react'

const requestRoute = getRouteApi('/$owner/$repo/requests/$requestId')

const loadDiscussions = createServerFn({ method: 'GET' })
  .validator(parseLoadDiscussionsInput)
  .handler(({ data }) => loadRequestDiscussionsForRequest(data))

const loadReplies = createServerFn({ method: 'GET' })
  .validator(parseLoadRepliesInput)
  .handler(({ data }) => loadRequestDiscussionRepliesForRequest(data))

const createDiscussion = createServerFn({ method: 'POST' })
  .validator(parseCreateDiscussionInput)
  .handler(({ data }) => createRequestDiscussionForRequest(data))

const createReply = createServerFn({ method: 'POST' })
  .validator(parseCreateReplyInput)
  .handler(({ data }) => createRequestDiscussionReplyForRequest(data))

const resolveDiscussion = createServerFn({ method: 'POST' })
  .validator(parseDiscussionActionInput)
  .handler(({ data }) => resolveRequestDiscussionForRequest(data))

const reopenAndReply = createServerFn({ method: 'POST' })
  .validator(parseCreateReplyInput)
  .handler(({ data }) => reopenAndReplyToRequestDiscussionForRequest(data))

const markDiscussionRead = createServerFn({ method: 'POST' })
  .validator(parseMarkDiscussionReadInput)
  .handler(({ data }) => markRequestDiscussionReadForRequest(data))

export const Route = createFileRoute('/$owner/$repo/requests/$requestId/_discussion/')({
  pendingComponent: RequestDiscussionPending,
  component: RequestDiscussionRoute,
})

function RequestDiscussionRoute() {
  const page = useRequestState()
  const { discussionPage, initial } = requestRoute.useLoaderData()
  const params = Route.useParams()
  const search = Route.useSearch()
  const live = useRepoLayout()

  if (!page.state) return null
  const detail = page.state.detail
  const viewer = page.state.viewer
  if (initial.scope !== page.scope) return <RequestDiscussionPending />
  const renderDiscussion = (initialPage: RequestDiscussionPage | null) => !initialPage ? (
    <section className="px-5 py-14 text-center lg:px-7">
      <MessageSquare className="mx-auto size-5 text-muted-foreground" />
      <h2 className="mt-3 text-sm font-semibold">Discussion is unavailable</h2>
      <p className="mx-auto mt-1 max-w-md text-sm leading-6 text-muted-foreground">
        The request is still available. Reload the page to try loading its discussion again.
      </p>
    </section>
  ) : (
    <RequestDiscussionView
      viewer={viewer}
      viewerId={page.viewerId}
      createDiscussion={(data) => createDiscussion({ data })}
      createReply={(data) => createReply({ data })}
      detail={detail}
      focusedDiscussionId={search.discussion}
      initialPage={initialPage}
      live={live}
      loadDiscussions={(data) => loadDiscussions({ data })}
      loadDiscussionChanges={(data) => loadRequestDiscussionChanges({ data })}
      loadReplies={(data) => loadReplies({ data })}
      markDiscussionRead={(data) => markDiscussionRead({ data })}
      params={{ owner: params.owner, repo: params.repo }}
      reopenAndReply={(data) => reopenAndReply({ data })}
      resolveDiscussion={(data) => resolveDiscussion({ data })}
    />
  )
  return discussionPage instanceof Promise ? (
    <Suspense fallback={<RequestDiscussionPending />}>
      <Await promise={discussionPage}>{renderDiscussion}</Await>
    </Suspense>
  ) : renderDiscussion(discussionPage)
}
