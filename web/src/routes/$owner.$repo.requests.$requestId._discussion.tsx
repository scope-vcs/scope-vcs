import {
  parseRequestParams,
  parseAuthorizeRequestAutoMergeInput,
  parseCancelRequestAutoMergeInput,
  parseUpdateDescriptionInput,
  parseRequestActionInput,
  parseRateRequestInput,
} from '@/api/request-inputs'
import {
  approveRequestChecks,
  getRequestChecks,
  rateRequestForRequest,
  type RateRequestInput,
} from '@/api/requests'
import {
  authorizeRequestAutoMergeForRequest,
  cancelRequestAutoMergeForRequest,
  loadRequestAutoMergeForRequest,
} from '@/features/requests/request-auto-merge-api'
import {
  type RequestActionCommand,
  performRequestActionForRequest,
} from '@/features/requests/request-actions-api'
import {
  loadRequestActivityForRequest,
  updateRequestDescriptionForRequest,
} from '@/features/requests/request-discussion-api'
import { RequestDetailPage } from '@/features/requests/request-detail-page'
import { RequestDetailPagePending } from '@/features/requests/request-page-pending'
import { requestParamsForRoute } from '@/features/requests/request-route-data'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { requestAttachmentActions } from '@/routes/-request-attachment-actions'
import { createFileRoute, getRouteApi, Outlet, useRouter } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useCallback, useMemo } from 'react'

const requestRoute = getRouteApi('/$owner/$repo/requests/$requestId')

const loadActivity = createServerFn({ method: 'GET' })
  .validator(parseRequestParams)
  .handler(({ data }) => loadRequestActivityForRequest(data))

const loadChecks = createServerFn({ method: 'GET' })
  .validator(parseRequestParams)
  .handler(({ data }) => getRequestChecks(data))

const approveChecks = createServerFn({ method: 'POST' })
  .validator(parseRequestParams)
  .handler(({ data }) => approveRequestChecks(data))

const loadAutoMerge = createServerFn({ method: 'GET' })
  .validator(parseRequestParams)
  .handler(({ data }) => loadRequestAutoMergeForRequest(data))

const authorizeAutoMerge = createServerFn({ method: 'POST' })
  .validator(parseAuthorizeRequestAutoMergeInput)
  .handler(({ data }) => authorizeRequestAutoMergeForRequest(data))

const cancelAutoMerge = createServerFn({ method: 'POST' })
  .validator(parseCancelRequestAutoMergeInput)
  .handler(({ data }) => cancelRequestAutoMergeForRequest(data))

const updateDescription = createServerFn({ method: 'POST' })
  .validator(parseUpdateDescriptionInput)
  .handler(({ data }) => updateRequestDescriptionForRequest(data))

const runRequestAction = createServerFn({ method: 'POST' })
  .validator(parseRequestActionInput)
  .handler(({ data }) => performRequestActionForRequest(data))

const rateRequest = createServerFn({ method: 'POST' })
  .validator(parseRateRequestInput)
  .handler(({ data }) => rateRequestForRequest(data))

// The request page: header, checks, description and details around the
// discussion. The changes screen is a sibling with its own layout.
export const Route = createFileRoute('/$owner/$repo/requests/$requestId/_discussion')({
  pendingComponent: RequestDetailPagePending,
  component: RequestDiscussionLayout,
})

function RequestDiscussionLayout() {
  const params = Route.useParams()
  const page = requestRoute.useLoaderData()
  const live = useRepoLayout()
  const router = useRouter()
  const navigate = Route.useNavigate()
  const repoParams = useMemo(
    () => ({ owner: params.owner, repo: params.repo }),
    [params.owner, params.repo],
  )
  const requestParams = useMemo(
    () => requestParamsForRoute({
      owner: params.owner,
      repo: params.repo,
      requestId: params.requestId,
    }),
    [params.owner, params.repo, params.requestId],
  )
  const performAction = useCallback(async (command: RequestActionCommand) => {
    const result = await runRequestAction({ data: { ...requestParams, ...command } })
    try {
      if (result.deleted) {
        await navigate({ params: repoParams, to: '/$owner/$repo/requests' })
      } else {
        await router.invalidate()
      }
      return result
    } catch {
      return {
        ...result,
        synchronizationError: 'The update completed, but the latest request state could not be reloaded. Refresh this page.',
      }
    }
  }, [navigate, repoParams, requestParams, router])
  const rateParticipant = useCallback(async (input: RateRequestInput) => {
    const rating = await rateRequest({ data: input })
    await router.invalidate()
    return rating
  }, [router])

  // The parent route renders the unavailable page for these.
  if (!page.detail || !page.ratings) return null

  return (
    <RequestDetailPage
      approveChecks={() => approveChecks({ data: requestParams })}
      authorizeAutoMerge={(input) =>
        authorizeAutoMerge({
          data: { ...requestParams, ...input },
        })}
      attachmentActions={requestAttachmentActions}
      cancelAutoMerge={(input) => cancelAutoMerge({
        data: { ...requestParams, ...input },
      })}
      detail={page.detail}
      live={live}
      loadActivity={(signal) => loadActivity({ data: requestParams, signal })}
      loadChecks={(signal) => loadChecks({ data: requestParams, signal })}
      loadAutoMerge={(signal) => loadAutoMerge({ data: requestParams, signal })}
      params={repoParams}
      performAction={performAction}
      ratings={page.ratings}
      rateRequest={rateParticipant}
      updateDescription={async (data) => {
        try {
          return await updateDescription({ data })
        } catch (error) {
          // Reload the server description so the preserved draft has an explicit recovery path.
          await router.invalidate().catch(() => {})
          throw error
        }
      }}
      viewerId={page.account?.user?.id ?? 'anonymous'}
    >
      <Outlet />
    </RequestDetailPage>
  )
}
