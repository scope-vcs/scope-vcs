import { EmptyState } from '@/components/empty-state'
import { Button } from '@/components/ui/button'
import { ChildRoutesPending } from '@/components/child-routes-pending'
import { RequestUnavailablePage } from '@/features/requests/request-detail-page'
import { RequestDetailPagePending } from '@/features/requests/request-page-pending'
import { RequestStateProvider, useRequestState } from '@/features/requests/request-state-context'
import { requestParamsForRoute } from '@/features/requests/request-route-data'
import { loadRequestStateValue, requestStateIdentity } from '@/features/requests/request-state-resource'
import { loadRepoSummary, repoSummaryIdentity } from '@/features/repo-detail/repo-summary-resource'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import { getCurrentViewerId } from '@/lib/viewer-state'
import { loadRequestChildrenForViewer, loadRequestState, loadRequestStateForViewer } from '@/routes/-request-state-actions'
import { loadRequestDiscussionRoute } from '@/routes/-request-discussion-loader'
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
    const live = await loadRepoSummary(
      repoSummaryIdentity(repoParams),
      (signal) => loadRepoSummaryState(repoParams, signal),
    )
    const activeViewerId = getCurrentViewerId()
    const answered = activeViewerId === undefined ? await loadRequestState({ data }) : null
    const viewerId = answered ? answered.viewerId : activeViewerId ?? null
    const scope = repoResourceScope(live.repo, viewerId)
    const discussionAvailable = location.pathname.endsWith('/changes')
      ? undefined
      : loadRequestDiscussionRoute(data, scope, viewerId, deps.discussion)
    await loadRequestStateValue(
      requestStateIdentity(scope, data.request_id),
      answered
        ? () => Promise.resolve({ state: answered.state })
        : (signal) => loadRequestStateForViewer(data, viewerId, signal),
      (cursor, signal) => loadRequestChildrenForViewer({ ...data, cursor }, viewerId, signal),
    )
    return { scope, viewerId, discussionAvailable }
  },
  pendingComponent: RequestRoutePending,
  component: RequestRoute,
})

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
