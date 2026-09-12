import { RepoContentError } from '@/components/repo-content-error'
import { RequestsPage } from '@/features/requests/requests-page'
import { Outlet, createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/requests')({
  errorComponent: RepoContentError,
  component: RequestsRoute,
})

function RequestsRoute() {
  return <RequestsPage params={Route.useParams()}><Outlet /></RequestsPage>
}
