import { isNotFoundError } from '@/api/http'
import { loadOwnerProfileForRequest } from '@/api/profile'
import { RouteErrorPage } from '@/components/route-error-page'
import { OwnerProfilePage } from '@/features/home/owner-profile-page'
import { OwnerProfilePending } from '@/features/home/owner-profile-pending'
import { createFileRoute, notFound } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadOwnerProfile = createServerFn({ method: 'GET' })
  .validator((input: { owner: string }) => input)
  .handler(async ({ data }) => {
    try {
      return await loadOwnerProfileForRequest(data.owner)
    } catch (error) {
      if (isNotFoundError(error)) throw notFound()
      throw error
    }
  })

export const Route = createFileRoute('/$owner/')({
  loader: ({ params }) => loadOwnerProfile({ data: params }),
  errorComponent: ({ error }) => (
    <RouteErrorPage
      error={error}
      fallbackMessage="Unexpected profile error"
      title="Profile unavailable"
    />
  ),
  notFoundComponent: () => (
    <RouteErrorPage
      error={new Error('No user exists with that handle.')}
      fallbackMessage="User not found"
      title="Profile not found"
    />
  ),
  pendingComponent: OwnerProfileRoutePending,
  component: OwnerProfileRoute,
})

function OwnerProfileRoutePending() {
  const { owner } = Route.useParams()
  return <OwnerProfilePending owner={owner} />
}

function OwnerProfileRoute() {
  return <OwnerProfilePage state={Route.useLoaderData()} />
}
