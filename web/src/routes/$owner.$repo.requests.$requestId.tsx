import {
  parseRequestParams,
  parseUpdateDescriptionInput,
  parseRequestActionInput,
  parseRateRequestInput,
} from '@/api/request-inputs'
import { createApiClient } from '@/api/client'
import { loadOptionalResource } from '@/api/http'
import { ApiRouteTemplates, buildApiPath } from '@/api/types.generated'
import { apiValidators } from '@/api/validators.generated'
import { loadRequestForRequest } from '@/api/requests'
import {
  loadRequestRatingsForRequest,
  rateRequestForRequest,
  type RateRequestInput,
} from '@/api/requests'
import {
  type RequestActionCommand,
  performRequestActionForRequest,
} from '@/features/requests/request-actions-api'
import {
  loadRequestActivityForRequest,
  updateRequestDescriptionForRequest,
} from '@/features/requests/request-discussion-api'
import {
  RequestDetailPage,
  RequestUnavailablePage,
} from '@/features/requests/request-detail-page'
import { RequestDetailPagePending } from '@/features/requests/request-page-pending'
import { requestParamsForRoute } from '@/features/requests/request-route-data'
import { useRepoLayout } from '@/features/repo-detail/repo-layout-context'
import type { RequestChangesSearch } from '@/features/requests/request-changes-workbench'
import { parseRouteFilePathSearch } from '@/lib/route-file'
import { requestAttachmentActions } from '@/routes/-request-attachment-actions'
import { createFileRoute, Outlet, useRouter } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { useCallback, useMemo } from 'react'

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
      loadOptionalResource(() => createApiClient().get(
        buildApiPath(ApiRouteTemplates.accountSession),
        apiValidators.AccountSessionResponse,
        { auth: 'optional' },
      )),
      loadOptionalResource(() => loadRequestRatingsForRequest(requestParams)),
    ])
    return {
      account,
      detail,
      ratings,
    }
  })

const loadActivity = createServerFn({ method: 'GET' })
  .validator(parseRequestParams)
  .handler(({ data }) => loadRequestActivityForRequest(data))

const updateDescription = createServerFn({ method: 'POST' })
  .validator(parseUpdateDescriptionInput)
  .handler(({ data }) => updateRequestDescriptionForRequest(data))

const runRequestAction = createServerFn({ method: 'POST' })
  .validator(parseRequestActionInput)
  .handler(({ data }) => performRequestActionForRequest(data))

const rateRequest = createServerFn({ method: 'POST' })
  .validator(parseRateRequestInput)
  .handler(({ data }) => rateRequestForRequest(data))

export const Route = createFileRoute('/$owner/$repo/requests/$requestId')({
  validateSearch: parseRequestDetailSearch,
  loader: ({ params }) => loadRequestPage({ data: requestParamsForRoute(params) }),
  pendingComponent: RequestDetailPagePending,
  component: RequestRoute,
})

function RequestRoute() {
  const params = Route.useParams()
  const page = Route.useLoaderData()
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

  if (!page.detail || !page.ratings) {
    return <RequestUnavailablePage params={repoParams} />
  }

  return (
    <RequestDetailPage
      attachmentActions={requestAttachmentActions}
      detail={page.detail}
      live={live}
      loadActivity={(signal) => loadActivity({ data: requestParams, signal })}
      params={repoParams}
      performAction={performAction}
      ratings={page.ratings}
      rateRequest={rateParticipant}
      updateDescription={(data) => updateDescription({ data })}
      viewerId={page.account?.user?.id ?? 'anonymous'}
    >
      <Outlet />
    </RequestDetailPage>
  )
}

export type RequestDetailSearch = RequestChangesSearch & {
  discussion?: string
}

function parseRequestDetailSearch(
  search: Record<string, unknown>,
): RequestDetailSearch {
  return {
    commit: searchText(search.commit),
    discussion: searchText(search.discussion),
    path: parseRouteFilePathSearch(search.path),
    revision: searchText(search.revision),
  }
}

function searchText(value: unknown) {
  return typeof value === 'string' && value.trim() ? value.trim() : undefined
}
