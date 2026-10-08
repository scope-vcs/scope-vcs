import { EmptyState } from '@/components/empty-state'
import { Button } from '@/components/ui/button'
import { ChildRoutesPending } from '@/components/child-routes-pending'
import { RequestUnavailablePage } from '@/features/requests/request-detail-page'
import { RequestDetailPagePending } from '@/features/requests/request-page-pending'
import { RequestStateProvider, useRequestState } from '@/features/requests/request-state-context'
import { requestParamsForRoute } from '@/features/requests/request-route-data'
import { loadRequestStateValue, requestRouteState, requestStateIdentity } from '@/features/requests/request-state-resource'
import { loadRequestDiscussionSession, requestDiscussionCacheKey } from '@/features/requests/request-discussion-cache'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import { getCurrentViewerId } from '@/lib/viewer-state'
import { loadRequestState } from '@/routes/-request-state-actions'
import { loadRequestDiscussionPage, loadRequestDiscussionChanges } from '@/routes/-request-discussion-actions'
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
    const loadDiscussion = () => loadRequestDiscussionPage({ data: { ...data, discussion_id: deps.discussion } })
    const discussion = location.pathname.endsWith('/changes') ? null : scope
      ? loadRequestDiscussionSession({
          key: requestDiscussionCacheKey({ repoId: scope, requestId: data.request_id, viewerId: viewerId ?? 'anonymous' }),
          focusedDiscussionId: deps.discussion,
          load: loadDiscussion,
          loadChanges: (after) => loadRequestDiscussionChanges({ data: { ...data, after } }),
        })
      : loadDiscussion()
    const discussionPage = discussion instanceof Promise ? discussion.catch(() => null) : discussion
    const initial = scope
      ? requestRouteState(live, await loadRequestStateValue(requestStateIdentity(scope, data.request_id), async (signal) => {
          const loaded = await loadRequestState({ data, signal })
          if (loaded.viewerId !== viewerId) throw new Error('The account changed while loading this request.')
          return { state: loaded.state }
        }), viewerId ?? null)
      : await loadRequestState({ data }).then((loaded) => requestRouteState(live, loaded, loaded.viewerId))
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
