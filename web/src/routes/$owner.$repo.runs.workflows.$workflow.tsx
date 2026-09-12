import { RepositoryRunsRoute } from '@/features/runs/repository-runs-route'
import { RunsPagePending } from '@/features/runs/runs-page-pending'
import { RunsPageError } from '@/features/runs/runs-page-error'
import { loadRepoRunPage } from '@/routes/-run-history-actions'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/runs/workflows/$workflow')({
  loader: ({ params }) => loadRepoRunPage({ data: params }),
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
