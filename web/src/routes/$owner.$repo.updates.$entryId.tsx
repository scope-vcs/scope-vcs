import { UpdatePage } from '@/features/history/update-page'
import { UpdatePagePending } from '@/features/history/update-page-pending'
import { parseUpdateSearch } from '@/features/history/update-search'
import { RouteErrorContent } from '@/components/route-error-page'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/updates/$entryId')({
  validateSearch: parseUpdateSearch,
  errorComponent: ({ error }) => (
    <RouteErrorContent
      error={error}
      fallbackMessage="Unexpected update error"
      title="Update unavailable"
    />
  ),
  pendingComponent: UpdatePagePending,
  component: UpdateRoute,
})

function UpdateRoute() {
  return (
    <UpdatePage
      params={Route.useParams()}
      search={Route.useSearch()}
    />
  )
}
