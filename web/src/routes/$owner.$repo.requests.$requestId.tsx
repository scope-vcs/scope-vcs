import { EmptyState } from '@/components/empty-state'
import { Button } from '@/components/ui/button'
import { ChildRoutesPending } from '@/components/child-routes-pending'
import { RequestUnavailablePage } from '@/features/requests/detail/request-detail-page'
import { RequestDetailPagePending } from '@/features/requests/detail/request-page-pending'
import { RequestStateProvider, useRequestState } from '@/features/requests/detail/request-state-context'
import { requestParamsForRoute } from '@/features/requests/detail/request-route-data'
import { loadRequestStateValue, requestStateIdentity } from '@/features/requests/detail/request-state-resource'
import {
  loadRequestAttachmentValue,
  requestAttachmentResource,
  requestAttachmentResourceIdentity,
  warmRequestAttachments,
} from '@/features/requests/attachments/request-attachment-resource'
import { loadRepoSummary, repoSummaryIdentity, retainedRepoSummary } from '@/features/repo-detail/repo-summary-resource'
import { repoChangeGeneration } from '@/features/repo-detail/repo-resource'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import type { RequestParams } from '@/api/types'
import { getCurrentViewerId } from '@/lib/viewer-state'
import { requestAttachmentActions } from '@/routes/-request-attachment-actions'
import {
  loadRequestChildrenForViewer,
  loadRequestState,
  loadRequestStateForViewer,
  requestStateForViewer,
} from '@/routes/-request-state-actions'
import { loadRequestDiscussionFirstPage, loadRequestDiscussionRoute } from '@/routes/-request-discussion-loader'
import { loadRepoSummaryState } from '@/routes/-repo-summary-actions'
import { createFileRoute, Outlet } from '@tanstack/react-router'
import { useMemo } from 'react'

export const Route = createFileRoute('/$owner/$repo/requests/$requestId')({
  validateSearch: (search: Record<string, unknown>): { discussion?: string } => ({
    discussion: typeof search.discussion === 'string' && search.discussion.trim()
      ? search.discussion.trim()
      : undefined,
  }),
  loaderDeps: ({ search }) => ({ discussion: search.discussion }),
  loader: async ({ deps, params, location }) => {
    const data = requestParamsForRoute(params)
    const repoParams = { owner: params.owner, repo: params.repo }
    const summaryIdentity = repoSummaryIdentity(repoParams)
    const showsDiscussion = !location.pathname.endsWith('/changes')
    const activeViewerId = getCurrentViewerId()
    const generation = repoChangeGeneration()
    const unchanged = () => repoChangeGeneration() === generation
    const started = retainedRepoSummary(summaryIdentity) === null
      ? startRequestPageLoads(data, showsDiscussion, deps.discussion)
      : null
    const live = await loadRepoSummary(summaryIdentity, (signal) => loadRepoSummaryState(repoParams, signal))
    const answered = activeViewerId === undefined ? await (started?.state ?? loadRequestState({ data })) : null
    const viewerId = answered ? answered.viewerId : activeViewerId ?? null
    const scope = repoResourceScope(live.repo, viewerId)
    const attachmentsIdentity = requestAttachmentResourceIdentity(scope, data.request_id)
    const attachments = started
      ? started.attachments
      : requestAttachmentResource.isCurrent(attachmentsIdentity) ? null : startRequestAttachments(data)
    const discussionAvailable = showsDiscussion
      ? loadRequestDiscussionRoute(data, scope, viewerId, deps.discussion, started?.discussion)
      : undefined
    const { state } = await loadRequestStateValue(
      requestStateIdentity(scope, data.request_id),
      answered && unchanged()
        ? () => Promise.resolve({ state: answered.state })
        : started && unchanged()
          ? () => requestStateForViewer(started.state, viewerId)
          : (signal) => loadRequestStateForViewer(data, viewerId, signal),
      (cursor, signal) => loadRequestChildrenForViewer({ ...data, cursor }, viewerId, signal),
    )
    if (state && attachments && unchanged()) {
      warmRequestAttachments(attachmentsIdentity, attachments)
    }
    return { scope, viewerId, discussionAvailable }
  },
  pendingComponent: RequestRoutePending,
  component: RequestRoute,
})

function startRequestPageLoads(data: RequestParams, showsDiscussion: boolean, discussionId: string | undefined) {
  const started = {
    state: loadRequestState({ data }),
    attachments: startRequestAttachments(data),
    discussion: showsDiscussion ? loadRequestDiscussionFirstPage(data, discussionId) : undefined,
  }
  void started.state.catch(() => {})
  void started.discussion?.catch(() => {})
  return started
}

function startRequestAttachments(data: RequestParams) {
  const attachments = loadRequestAttachmentValue(requestAttachmentActions, data)
  void attachments.catch(() => {})
  return attachments
}

function RequestRoutePending() {
  return <ChildRoutesPending below="/$owner/$repo/requests/$requestId" />
}

function RequestRoute() {
  const params = Route.useParams()
  const page = Route.useLoaderData()
  const requestParams = useMemo(() => requestParamsForRoute({
    owner: params.owner,
    repo: params.repo,
    requestId: params.requestId,
  }), [params.owner, params.repo, params.requestId])
  return (
    <RequestStateProvider loadedViewerId={page.viewerId} params={requestParams}>
      <RequestRouteContent params={{ owner: params.owner, repo: params.repo }} />
    </RequestStateProvider>
  )
}

function RequestRouteContent({ params }: { params: { owner: string; repo: string } }) {
  const page = useRequestState()
  if (page.unavailable) return <RequestUnavailablePage params={params} />
  if (!page.state && page.error) return (
    <EmptyState title="Request unavailable" description={page.error} action={<Button onClick={page.retry} variant="secondary">Try again</Button>} />
  )
  if (!page.state) return <RequestDetailPagePending />
  return <Outlet />
}
