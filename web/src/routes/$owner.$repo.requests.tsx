import { repoViews } from '@/api/repo-views'
import type { RepoParams } from '@/api/types'
import type { ViewId } from '@/api/types.generated'
import { RepoContentError } from '@/components/repo-content-error'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import { loadRepoSummary, repoSummaryIdentity } from '@/features/repo-detail/repo-summary-resource'
import { resolveViewingAs } from '@/features/repo-detail/viewing-as'
import { requestQueueIdentity, warmRequestQueue } from '@/features/requests/workspace/request-queue-cache'
import { RequestsPage } from '@/features/requests/workspace/requests-page'
import { RequestsPagePending } from '@/features/requests/workspace/requests-page-pending'
import { whenViewerActive } from '@/lib/viewer-state'
import { loadRepoSummaryState } from '@/routes/-repo-summary-actions'
import { requestQueuePageLoader } from '@/routes/-request-workspace-actions'
import { Outlet, createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/requests')({
  loaderDeps: ({ search }) => ({ view: search.view }),
  loader: ({ deps, params }) => {
    void warmRequestsPageQueue({ owner: params.owner, repo: params.repo }, deps.view).catch(() => {})
  },
  errorComponent: RepoContentError,
  pendingComponent: RequestsPagePending,
  component: RequestsRoute,
})

async function warmRequestsPageQueue(params: RepoParams, requested: ViewId | undefined) {
  const live = await loadRepoSummary(repoSummaryIdentity(params), (signal) => loadRepoSummaryState(params, signal))
  const viewerId = await whenViewerActive()
  const view = resolveViewingAs(repoViews(live.repo.views), live.repo.access.view, requested)
  warmRequestQueue(
    requestQueueIdentity(repoResourceScope(live.repo, viewerId), view),
    String(live.repo.change_version),
    requestQueuePageLoader(params, view),
  )
}

function RequestsRoute() {
  return <RequestsPage params={Route.useParams()}><Outlet /></RequestsPage>
}
