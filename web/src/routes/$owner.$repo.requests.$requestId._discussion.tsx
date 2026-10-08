import {
  parseRequestParams,
  parseApproveRequestChecksInput,
  parseAuthorizeRequestAutoMergeInput,
  parseCancelRequestAutoMergeInput,
  parseUpdateDescriptionInput,
  parseRequestActionInput,
  parseRateRequestInput,
} from '@/api/request-inputs'
import {
  approveRequestChecks,
  rateRequestForRequest,
  loadRequestRatingsForRequest,
  type RateRequestInput,
} from '@/api/requests'
import {
  authorizeRequestAutoMergeForRequest,
  cancelRequestAutoMergeForRequest,
} from '@/features/requests/request-auto-merge-api'
import {
  type RequestActionCommand,
  performRequestActionForRequest,
} from '@/features/requests/request-actions-api'
import {
  loadRequestActivityForRequest,
  updateRequestDescriptionForRequest,
} from '@/features/requests/request-discussion-api'
import { useRequestState } from '@/features/requests/request-state-context'
import { requestRatingsResource } from '@/features/requests/request-ratings-resource'
import { reconcileRequestState, requestStateResource } from '@/features/requests/request-state-resource'
import { RequestDetailPage } from '@/features/requests/request-detail-page'
import { RequestDetailPagePending } from '@/features/requests/request-page-pending'
import { requestParamsForRoute } from '@/features/requests/request-route-data'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import { requestAttachmentActions } from '@/routes/-request-attachment-actions'
import { createFileRoute, Outlet, useRouter } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useCallback, useMemo } from 'react'

const loadActivity = createServerFn({ method: 'GET' })
  .validator(parseRequestParams)
  .handler(({ data }) => loadRequestActivityForRequest(data))

const approveChecks = createServerFn({ method: 'POST' })
  .validator(parseApproveRequestChecksInput)
  .handler(({ data }) => approveRequestChecks(data))

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

const loadRatings = createServerFn({ method: 'GET' })
  .validator(parseRequestParams)
  .handler(({ data }) => loadRequestRatingsForRequest(data))

const rateRequest = createServerFn({ method: 'POST' })
  .validator(parseRateRequestInput)
  .handler(({ data }) => rateRequestForRequest(data))

export const Route = createFileRoute('/$owner/$repo/requests/$requestId/_discussion')({
  pendingComponent: RequestDetailPagePending,
  component: RequestDiscussionLayout,
})

function RequestDiscussionLayout() {
  const params = Route.useParams()
  const page = useRequestState()
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
    let result
    try {
      result = await runRequestAction({ data: { ...requestParams, ...command } })
    } catch (error) {
      requestStateResource.invalidate(page.identity)
      await router.invalidate({ sync: true }).catch(() => {})
      throw error
    }
    try {
      if (result.deleted) {
        requestStateResource.removeMatching((identity) => identity === page.identity)
        await navigate({ params: repoParams, to: '/$owner/$repo/requests' })
      } else {
        requestStateResource.invalidate(page.identity)
        await router.invalidate({ sync: true })
      }
      return result
    } catch {
      return {
        ...result,
        synchronizationError: 'The update completed, but the latest request state could not be reloaded. Refresh this page.',
      }
    }
  }, [navigate, repoParams, requestParams, router, page.identity])
  const rateParticipant = useCallback(async (input: RateRequestInput) => {
    const rating = await rateRequest({ data: input })
    requestRatingsResource.invalidate(page.identity)
    return rating
  }, [page.identity])

  if (!page.state) return null

  return (
    <RequestDetailPage
      approveChecks={(expectedHeadOid) =>
        approveChecks({ data: { ...requestParams, expected_head_oid: expectedHeadOid } })}
      authorizeAutoMerge={(input) =>
        authorizeAutoMerge({
          data: { ...requestParams, ...input },
        })}
      attachmentActions={requestAttachmentActions}
      cancelAutoMerge={(input) => cancelAutoMerge({
        data: { ...requestParams, ...input },
      })}
      state={page.state}
      stateError={page.error}
      identity={page.identity}
      scope={page.scope}
      live={live}
      loadActivity={(signal) => loadActivity({ data: requestParams, signal })}
      loadRatings={(signal) => loadRatings({ data: requestParams, signal })}
      params={repoParams}
      performAction={performAction}
      rateRequest={rateParticipant}
      updateDescription={async (data) => {
        const snapshot = requestStateResource.getSnapshot(page.identity)
        try {
          const result = await updateDescription({ data })
          reconcileRequestState(page.identity, snapshot, (current) => ({ ...current, detail: { request: result.request } }))
          requestStateResource.invalidate(page.identity)
          return result
        } catch (error) {
          requestStateResource.invalidate(page.identity)
          await router.invalidate().catch(() => {})
          throw error
        }
      }}
      viewerId={page.viewerId ?? 'anonymous'}
    >
      <Outlet />
    </RequestDetailPage>
  )
}
