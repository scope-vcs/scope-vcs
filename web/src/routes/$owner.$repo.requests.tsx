import { RepoContentError } from '@/components/repo-content-error'
import { RequestsPage } from '@/features/requests/workspace/requests-page'
import { RequestsPagePending } from '@/features/requests/workspace/requests-page-pending'
import { Outlet, createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/requests')({
  errorComponent: RepoContentError,
  pendingComponent: RequestsPagePending,
  component: RequestsRoute,
})

function RequestsRoute() {
  return <RequestsPage params={Route.useParams()}><Outlet /></RequestsPage>
}
