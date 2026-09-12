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
import { Badge } from '@/components/ui/badge'
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
import { RequestDetailsProvider } from './request-details'
import type { RequestActivityPage } from './request-discussion-types'
import { RequestDescription } from './request-description'
import type { UpdateDescriptionInput } from './request-discussion-api'
import {
  requestMergeabilityLabel,
  requestMergeabilityTone,
  requestStatusLabel,
  requestStatusTone,
} from './request-labels'
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

  async function saveDescription(nextDescription: string, expectedDescription: string) {
    try {
      await updateDescription({
        ...requestParams,
        description_markdown: nextDescription,
        expected_description_markdown: expectedDescription,
      })
      setDescriptionOverride({ server: serverDescription, value: nextDescription })
      return true
    } catch {
      return false
    }
  }

  function requestHeader() {
    return (
      <header className="border-b border-border px-5 pb-5 pt-6 sm:px-6 lg:px-8">
        <div className="flex flex-col gap-5 xl:flex-row xl:items-start xl:justify-between">
          <div className="min-w-0">
            <h1 className="break-words text-[26px] font-medium leading-[1.18] tracking-[-0.025em] sm:text-[29px]">
              {request.title}
            </h1>
            <div className="mt-3 flex flex-wrap items-center gap-2">
              <Badge variant={requestStatusTone(request)}>
                {requestStatusLabel(request)}
              </Badge>
              {request.state === 'Open' ? (
                <Badge variant={requestMergeabilityTone(request)}>
                  {requestMergeabilityLabel(request)}
                </Badge>
              ) : null}
              <span className="font-mono text-xs text-muted-foreground">
                {request.name}
              </span>
            </div>
            {workspace?.selected ? (
              <p className="mt-3 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
                <span className="font-medium text-foreground">{workspace.selected.author.handle}</span>
                <span>opened #{workspace.selected.request.id}</span>
                {workspace.selected.claimer ? (
                  <><span aria-hidden="true">·</span><span>Reviewing: {workspace.selected.claimer.handle}</span></>
                ) : null}
              </p>
            ) : null}
          </div>
          <div className="flex flex-wrap items-center gap-2 xl:justify-end">
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
            {workspace?.selected?.attention.reason === 'unclaimed' &&
            workspace.selected.attention.can_claim ? (
              <Button onClick={workspace.claim} size="sm" type="button" variant="secondary">
                <UserRound />
                I’ll take this
              </Button>
            ) : null}
            {workspace?.selected?.attention.can_release ? (
              <Button onClick={workspace.release} size="sm" type="button" variant="secondary">
                <UserRoundMinus />
                Release
              </Button>
            ) : null}
            <RequestLifecycleActions
              actions={requestActions}
              className="fixed inset-x-0 bottom-0 z-30 flex flex-wrap justify-end border-t border-border bg-background px-3 py-3 pb-[max(0.75rem,env(safe-area-inset-bottom))] min-[701px]:static min-[701px]:border-0 min-[701px]:bg-transparent min-[701px]:p-0"
              request={request}
            />
            {request.permissions.can_view_activity ? (
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
          </div>
        </div>
        {requestActions.error ? (
          <p className="mt-3 text-sm text-danger-strong" role="alert">
            {requestActions.error}
          </p>
        ) : null}
      </header>
    )
  }

  return (
    <RequestAttachmentProvider
      actions={attachmentActions}
      live={live}
      requestId={request.id}
      viewerId={viewerId}
    >
      <WorkbenchPane className="max-w-none">
        <div
          className={cn(
            'mx-auto w-full max-w-[1180px]',
            hasLifecycleActions && 'pb-20 min-[701px]:pb-0',
          )}
        >
          {requestHeader()}
          <div className="min-h-0 pt-4">
            <RequestDescription
              canEdit={request.permissions.can_edit_identity}
              description={description}
              onSave={saveDescription}
            />
            <RequestViewTabs params={{ ...params, requestId: request.id }} />
            <RequestDetailsProvider value={{
              actions: requestActions,
              onRate: rateRequest,
              params: requestParams,
              ratings,
              request,
            }}>
              <div className="min-w-0">{children}</div>
            </RequestDetailsProvider>
          </div>

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
        activeProps={{ className: 'border-brand text-foreground' }}
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
        activeProps={{ className: 'border-brand text-foreground' }}
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
        activeProps={{ className: 'border-brand text-foreground' }}
        className={tabClass}
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
