import { RequestDetails } from '@/features/requests/request-details'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/requests/$requestId/details')({
  component: RequestDetailsRoute,
})

/** From 1400px the detail page renders the rail, so the tab body stands down. */
function RequestDetailsRoute() {
  return (
    <div className="border-t border-border min-[1400px]:hidden">
      <RequestDetails />
    </div>
  )
}
