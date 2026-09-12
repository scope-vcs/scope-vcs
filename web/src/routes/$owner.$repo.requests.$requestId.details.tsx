import { RequestDetails } from '@/features/requests/request-details'
import { createFileRoute } from '@tanstack/react-router'

export const Route = createFileRoute('/$owner/$repo/requests/$requestId/details')({
  component: RequestDetails,
})
