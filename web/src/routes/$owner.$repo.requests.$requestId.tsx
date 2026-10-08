import { EmptyState } from '@/components/empty-state'
import { Button } from '@/components/ui/button'
import { ChildRoutesPending } from '@/components/child-routes-pending'
import { RequestUnavailablePage } from '@/features/requests/request-detail-page'
import { RequestDetailPagePending } from '@/features/requests/request-page-pending'
import { RequestStateProvider, useRequestState } from '@/features/requests/request-state-context'
import { requestParamsForRoute } from '@/features/requests/request-route-data'
import { loadRequestStateValue, requestRouteState, requestStateIdentity } from '@/features/requests/request-state-resource'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import { getCurrentViewerId } from '@/lib/viewer-state'
import { loadRequestState, loadRequestStateForViewer } from '@/routes/-request-state-actions'
import { loadRequestDiscussionRoutePage } from '@/routes/-request-discussion-loader'
import { createFileRoute, Outlet } from '@tanstack/react-router'
import { useMemo } from 'react'

export const Route = createFileRoute('/$owner/$repo/requests/$requestId')({
  validateSearch: (search: Record<string, unknown>): { discussion?: string } => ({
    discussion: typeof search.discussion === 'string' && search.discussion.trim()
      ? search.discussion.trim()
      : undefined,
  }),
  loaderDeps: ({ search }) => ({ discussion: search.discussion }),
  loader: async ({ deps, params, parentMatchPromise, location }) => {
    const live = (await parentMatchPromise).loaderData
    if (!live) throw new Error('Repository state is unavailable.')
    const data = requestParamsForRoute(params)
    const viewerId = typeof window === 'undefined' ? undefined : getCurrentViewerId()
    const scope = viewerId === undefined ? null : repoResourceScope(live.repo, viewerId)
    const discussionPage = location.pathname.endsWith('/changes')
      ? undefined
      : loadRequestDiscussionRoutePage(data, scope, viewerId ?? null, deps.discussion)
    const initial = viewerId === undefined
      ? await loadRequestState({ data }).then((loaded) => requestRouteState(live, loaded, loaded.viewerId))
      : requestRouteState(live, await loadRequestStateValue(
          requestStateIdentity(repoResourceScope(live.repo, viewerId), data.request_id),
          (signal) => loadRequestStateForViewer(data, viewerId, signal),
        ), viewerId)
    return { initial, discussionPage }
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
    <RequestStateProvider initial={page.initial} params={requestParams}>
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
