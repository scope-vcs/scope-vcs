import { parseRequestParams } from '@/api/request-inputs'
import { loadOptionalResource } from '@/api/http'
import { loadAccountSessionForRequest } from '@/api/profile'
import { loadRequestForRequest, loadRequestRatingsForRequest } from '@/api/requests'
import { ChildRoutesPending } from '@/components/child-routes-pending'
import { RequestUnavailablePage } from '@/features/requests/request-detail-page'
import { requestParamsForRoute } from '@/features/requests/request-route-data'
import { createFileRoute, Outlet } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'

const loadRequestPage = createServerFn({ method: 'GET' })
  .validator(parseRequestParams)
  .handler(async ({ data }) => {
    const requestParams = {
      owner: data.owner,
      repo: data.repo,
      request_id: data.request_id,
    }
    const [detail, account, ratings] = await Promise.all([
      loadOptionalResource(() => loadRequestForRequest(requestParams)),
      loadOptionalResource(loadAccountSessionForRequest),
      loadOptionalResource(() => loadRequestRatingsForRequest(requestParams)),
    ])
    return {
      account,
      detail,
      ratings,
    }
  })

// The discussion page and the changes screen share this request; each child
// supplies its own layout and pending shape.
export const Route = createFileRoute('/$owner/$repo/requests/$requestId')({
  loader: ({ params }) => loadRequestPage({ data: requestParamsForRoute(params) }),
  pendingComponent: RequestRoutePending,
  component: RequestRoute,
})

function RequestRoutePending() {
  return <ChildRoutesPending below="/$owner/$repo/requests/$requestId" />
}

function RequestRoute() {
  const params = Route.useParams()
  const page = Route.useLoaderData()
  if (!page.detail || !page.ratings) {
    return <RequestUnavailablePage params={{ owner: params.owner, repo: params.repo }} />
  }
  return <Outlet />
}
