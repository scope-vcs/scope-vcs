import { RequestDetails } from '@/features/requests/request-details'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/requests/$requestId/details')({
  component: RequestDetailsRoute,
})

/** The tab placement stands down while the page shows the rail. */
function RequestDetailsRoute() {
  return <RequestDetails placement="tab" />
}
