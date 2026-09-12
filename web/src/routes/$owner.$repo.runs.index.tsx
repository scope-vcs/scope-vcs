import { RepositoryRunsRoute } from '@/features/runs/repository-runs-route'
import { RunsPagePending } from '@/features/runs/runs-page-pending'
import { RunsPageError } from '@/features/runs/runs-page-error'
import { loadRepoRunPage } from '@/routes/-run-history-actions'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/runs/')({
  loader: ({ params }) => loadRepoRunPage({ data: params }),
  errorComponent: RunsPageError,
  pendingComponent: RunsPagePending,
  component: RepoRunsRoute,
})

function RepoRunsRoute() {
  return (
    <RepositoryRunsRoute
      initialResources={Route.useLoaderData()}
      params={Route.useParams()}
    />
  )
}
