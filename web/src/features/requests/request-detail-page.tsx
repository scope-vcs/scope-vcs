import type { RepoLiveState, RepoParams } from '@/api/types'
import type {
  RequestDetailResponse,
  RequestMutationResponse,
  RequestRatingResponse,
  RequestRatingsResponse,
} from '@/api/types.generated'
import type { RateRequestInput } from '@/api/requests'
import { EmptyState } from '@/components/empty-state'
import { PageContent, WorkbenchPane } from '@/components/page-header'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import {
  ArrowLeft,
  GitCommit,
  History,
  MessageSquare,
  ShieldQuestion,
  SlidersHorizontal,
  UserRound,
  UserRoundMinus,
} from 'lucide-react'
import { type ReactNode, useMemo, useState } from 'react'
import { RequestActivityDrawer } from './request-activity-drawer'
import type {
  RequestActionCommand,
  RequestActionResult,
} from './request-actions-api'
import { RequestDetailHeader } from './request-detail-header'
import { RequestDetails, RequestDetailsProvider } from './request-details'
import type { RequestActivityPage } from './request-discussion-types'
import { RequestDescription } from './request-description'
import type { UpdateDescriptionInput } from './request-discussion-api'
import { RequestLifecycleActions } from './request-lifecycle-actions'
import { hasRequestLifecycleActions } from './request-lifecycle-model'
import { useRequestActions } from './use-request-actions'
import { useRequestActivityHistory } from './use-request-activity-history'
import { requestActivityIdentity } from './request-activity-resource'
import { repoResourceScope } from '../repo-detail/repo-resource-scope'
import {
  RequestAttachmentProvider,
  type RequestAttachmentActions,
} from './request-attachment-context'
import { useRequestWorkspace } from './request-workspace-context'
import './request-detail.css'

export function RequestUnavailablePage({ params }: { params: RepoParams }) {
  return (
    <PageContent>
      <EmptyState
        action={(
          <Button asChild size="sm" variant="secondary">
            <Link params={params} to="/$owner/$repo/requests">
              Back to requests
            </Link>
          </Button>
        )}
        description="It does not exist, or this account cannot see it. Sign in with an account that has access."
        icon={<ShieldQuestion />}
        title="Request not found"
      />
    </PageContent>
  )
}

type RequestDetailPageProps = {
  attachmentActions: RequestAttachmentActions
  children: ReactNode
  detail: RequestDetailResponse
  live: RepoLiveState
  loadActivity: (signal: AbortSignal) => Promise<RequestActivityPage>
  params: RepoParams
  performAction: (command: RequestActionCommand) => Promise<RequestActionResult>
  ratings: RequestRatingsResponse
  rateRequest: (input: RateRequestInput) => Promise<RequestRatingResponse>
  updateDescription: (input: UpdateDescriptionInput) => Promise<RequestMutationResponse>
  viewerId: string
}

export function RequestDetailPage(props: RequestDetailPageProps) {
  const {
    children,
    attachmentActions,
    detail,
    live,
    loadActivity,
    params,
    performAction,
    ratings,
    rateRequest,
    updateDescription,
    viewerId,
  } = props
  const { request } = detail
  const serverDescription = request.description_markdown
  const history = useRequestActivityHistory({
    identity: request.permissions.can_view_activity
      ? requestActivityIdentity(
          repoResourceScope(live.repo, viewerId === 'anonymous' ? null : viewerId),
          request.id,
        )
      : null,
    load: loadActivity,
    version: String(request.activity_version),
  })
  const requestActions = useRequestActions(performAction)
  const workspace = useRequestWorkspace()
  const [descriptionOverride, setDescriptionOverride] = useState<{
    server: string
    value: string
  } | null>(null)
  const description = descriptionOverride?.server === serverDescription
    ? descriptionOverride.value
    : serverDescription
  const requestParams = useMemo(() => ({
    owner: params.owner,
    repo: params.repo,
    request_id: request.id,
  }), [params.owner, params.repo, request.id])
  const hasLifecycleActions = hasRequestLifecycleActions(request)
  const canClaim = workspace?.selected?.attention.reason === 'unclaimed' &&
    workspace.selected.attention.can_claim
  const canRelease = workspace?.selected?.attention.can_release ?? false
  // Below 701px the bar still carries the back link, so it always renders there.
  const hasBarActions = canClaim || canRelease || hasLifecycleActions

  async function saveDescription(nextDescription: string, expectedDescription: string) {
    await updateDescription({
      ...requestParams,
      description_markdown: nextDescription,
      expected_description_markdown: expectedDescription,
    })
    setDescriptionOverride({ server: serverDescription, value: nextDescription })
    return true
  }

  return (
    <RequestAttachmentProvider
      actions={attachmentActions}
      live={live}
      requestId={request.id}
      viewerId={viewerId}
    >
      <WorkbenchPane className="max-w-none">
        <div className={cn('w-full', hasLifecycleActions && 'pb-20 min-[701px]:pb-0')}>
          <RequestDetailHeader
            actions={request.permissions.can_view_activity ? (
              <Button
                aria-label="View request activity"
                onClick={history.openHistory}
                size="icon-sm"
                title="View request activity"
                type="button"
                variant="secondary"
              >
                <History />
              </Button>
            ) : null}
            request={request}
          />
          <div
            className={cn(
              'request-detail-actions flex flex-wrap items-center gap-2 px-5 py-2.5 sm:px-6 lg:px-8',
              !hasBarActions && 'min-[701px]:hidden',
            )}
          >
            <Button
              asChild
              className="min-[701px]:hidden"
              size="icon-sm"
              variant="secondary"
            >
              <Link
                aria-label="Back to requests"
                params={params}
                to="/$owner/$repo/requests"
              >
                <ArrowLeft />
              </Link>
            </Button>
            {canClaim ? (
              <Button onClick={workspace?.claim} size="sm" type="button" variant="secondary">
                <UserRound />
                I’ll take this
              </Button>
            ) : null}
            {canRelease ? (
              <Button onClick={workspace?.release} size="sm" type="button" variant="secondary">
                <UserRoundMinus />
                Release
              </Button>
            ) : null}
            <RequestLifecycleActions
              actions={requestActions}
              className="ml-auto fixed inset-x-0 bottom-0 z-30 flex flex-wrap justify-end border-t border-border bg-background px-3 py-3 pb-[max(0.75rem,env(safe-area-inset-bottom))] min-[701px]:static min-[701px]:border-0 min-[701px]:bg-transparent min-[701px]:p-0"
              request={request}
            />
          </div>
          {requestActions.error ? (
            <p
              className="border-b border-border px-5 py-2 text-sm text-danger-strong sm:px-6 lg:px-8"
              role="alert"
            >
              {requestActions.error}
            </p>
          ) : null}
          <RequestDetailsProvider value={{
            actions: requestActions,
            onRate: rateRequest,
            params: requestParams,
            ratings,
            request,
          }}>
            <div className="min-[1400px]:grid min-[1400px]:grid-cols-[minmax(0,1fr)_300px]">
              <div
                className="request-detail-document pt-4"
                data-state={request.state}
              >
                <RequestDescription
                  canEdit={request.permissions.can_edit_identity}
                  description={description}
                  onSave={saveDescription}
                />
                <RequestViewTabs params={{ ...params, requestId: request.id }} />
                <div className="min-w-0">{children}</div>
              </div>
              <aside className="hidden min-w-0 border-l border-border min-[1400px]:block">
                <RequestDetails />
              </aside>
            </div>
          </RequestDetailsProvider>

          <RequestActivityDrawer
            activity={history.activity}
            error={history.error}
            load={history.retry}
            loading={history.loading}
            onOpenChange={history.onOpenChange}
            open={history.open}
          />
        </div>
      </WorkbenchPane>
    </RequestAttachmentProvider>
  )
}

function RequestViewTabs({
  params,
}: {
  params: RepoParams & { requestId: string }
}) {
  const tabClass = 'inline-flex h-11 items-center gap-2 border-b-2 px-1 text-sm font-medium transition-colors'
  return (
    <nav aria-label="Request views" className="flex gap-5 px-5 lg:gap-6 lg:px-7">
      <Link
        activeOptions={{ exact: true }}
        activeProps={{ className: 'border-foreground text-foreground' }}
        className={tabClass}
        inactiveProps={{ className: 'border-transparent text-muted-foreground hover:text-foreground' }}
        params={params}
        preload="intent"
        resetScroll={false}
        search={{}}
        to="/$owner/$repo/requests/$requestId"
      >
        <MessageSquare className="size-3.5" />
        Discussion
      </Link>
      <Link
        activeProps={{ className: 'border-foreground text-foreground' }}
        className={tabClass}
        inactiveProps={{ className: 'border-transparent text-muted-foreground hover:text-foreground' }}
        params={params}
        preload="intent"
        resetScroll={false}
        search={{}}
        to="/$owner/$repo/requests/$requestId/changes"
      >
        <GitCommit className="size-3.5" />
        Changes
      </Link>
      <Link
        activeProps={{ className: 'border-foreground text-foreground' }}
        className={cn(tabClass, 'min-[1400px]:hidden')}
        inactiveProps={{ className: 'border-transparent text-muted-foreground hover:text-foreground' }}
        params={params}
        preload="intent"
        resetScroll={false}
        search={{}}
        to="/$owner/$repo/requests/$requestId/details"
      >
        <SlidersHorizontal className="size-3.5" />
        Details
      </Link>
    </nav>
  )
}
