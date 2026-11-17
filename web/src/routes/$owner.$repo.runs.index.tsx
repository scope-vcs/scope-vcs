import { RepositoryRunsRoute } from '@/features/runs/repository-runs-route'
import { RunsPagePending } from '@/features/runs/runs-page-pending'
import { RunsPageError } from '@/features/runs/runs-page-error'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/runs/')({
  errorComponent: RunsPageError,
  pendingComponent: RunsPagePending,
  component: RepoRunsRoute,
})

function RepoRunsRoute() {
  return <RepositoryRunsRoute params={Route.useParams()} />
}
