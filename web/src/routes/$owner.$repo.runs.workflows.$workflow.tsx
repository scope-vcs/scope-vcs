import { RepositoryRunsRoute } from '@/features/runs/repository-runs-route'
import { RunsPagePending } from '@/features/runs/runs-page-pending'
import { RunsPageError } from '@/features/runs/runs-page-error'
import { loadRepoRunPage } from '@/routes/-run-history-actions'
import { auth } from '@clerk/tanstack-react-start/server'
import type { RepoLiveState } from '@/api/types'
import { repoResourceScope } from '@/features/repo-detail/repo-resource-scope'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/runs/workflows/$workflow')({
  loader: async ({ params, parentMatchPromise }) => {
    if (typeof window !== 'undefined') return null
    const live = (await parentMatchPromise).loaderData as RepoLiveState
    const { userId } = await auth()
    return { scope: repoResourceScope(live.repo, userId), resources: await loadRepoRunPage({ data: params }) }
  },
  errorComponent: RunsPageError,
  pendingComponent: RunsPagePending,
  component: RepoWorkflowRunsRoute,
})

function RepoWorkflowRunsRoute() {
  const params = Route.useParams()
  return (
    <RepositoryRunsRoute
      initialResources={Route.useLoaderData()}
      params={params}
      workflow={params.workflow}
    />
  )
}
