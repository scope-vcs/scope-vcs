import type { RepoLiveState, RepoParams } from '@/api/types'
import type {
  RequestChecksResponse,
  RequestDetailResponse,
  RequestMutationResponse,
  RequestAutoMergeResponse,
  RequestRatingResponse,
  RequestRatingsResponse,
} from '@/api/types.generated'
import type { RateRequestInput } from '@/api/requests'
import { EmptyState } from '@/components/empty-state'
import { PageContent } from '@/components/page-header'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import {
  ArrowLeft,
  CirclePlay,
  GitCommit,
  MessageSquare,
  ShieldQuestion,
  SlidersHorizontal,
  UserRound,
  UserRoundMinus,
} from 'lucide-react'
import { type CSSProperties, type ReactNode, useMemo, useRef, useState } from 'react'
import { RequestActivityDrawer } from './request-activity-drawer'
import type {
  RequestActionCommand,
  RequestActionResult,
} from './request-actions-api'
import type {
  AuthorizeRequestAutoMergeInput,
  CancelRequestAutoMergeInput,
} from './request-auto-merge-api'
import { RequestChecksSection } from './request-checks-section'
import { requestChecksIdentity } from './request-checks-resource'
import { requestAutoMergeIdentity } from './request-auto-merge-resource'
import { RequestDetailHeader } from './request-detail-header'
import { RequestDetails, RequestDetailsProvider } from './request-details'
import type { RequestActivityPage } from './request-discussion-types'
import { RequestDescription } from './request-description'
import type { UpdateDescriptionInput } from './request-discussion-api'
import { RequestLifecycleActions } from './request-lifecycle-actions'
import { withCurrentMergeability } from './request-lifecycle-model'
import { RequestMoreMenu } from './request-more-menu'
import { useDetailPaneRail } from './use-detail-pane-rail'
import { useElementHeight } from './use-element-height'
import { useRequestActions } from './use-request-actions'
import { useRequestActivityHistory } from './use-request-activity-history'
import { useRequestChecks } from './use-request-checks'
import { useRequestAutoMerge } from './use-request-auto-merge'
import { requestActivityIdentity } from './request-activity-resource'
import { repoResourceScope } from '../repo-detail/repo-resource-scope'
import { RequestRevisionPushesProvider } from './request-revision-row'
import { RequestSideDrawer } from './request-side-drawer'
import { requestRevisionPushes } from './request-timeline-items'
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
  approveChecks: () => Promise<RequestChecksResponse>
  authorizeAutoMerge: (input: Pick<
    AuthorizeRequestAutoMergeInput,
    'expected_head_oid' | 'expected_revision_id'
  >) => Promise<RequestAutoMergeResponse>
  attachmentActions: RequestAttachmentActions
  children: ReactNode
  detail: RequestDetailResponse
  initialActivity: RequestActivityPage | null
  live: RepoLiveState
  loadActivity: (signal: AbortSignal) => Promise<RequestActivityPage>
  loadChecks: (signal: AbortSignal) => Promise<RequestChecksResponse>
  loadAutoMerge: (signal: AbortSignal) => Promise<RequestAutoMergeResponse>
  params: RepoParams
  performAction: (command: RequestActionCommand) => Promise<RequestActionResult>
  cancelAutoMerge: (input: Pick<
    CancelRequestAutoMergeInput,
    'expected_intent_id'
  >) => Promise<RequestAutoMergeResponse>
  ratings: RequestRatingsResponse
  rateRequest: (input: RateRequestInput) => Promise<RequestRatingResponse>
  updateDescription: (input: UpdateDescriptionInput) => Promise<RequestMutationResponse>
  viewerId: string
}

export function RequestDetailPage(props: RequestDetailPageProps) {
  const {
    approveChecks,
    authorizeAutoMerge,
    cancelAutoMerge,
    children,
    attachmentActions,
    detail,
    initialActivity,
    live,
    loadActivity,
    loadChecks,
    loadAutoMerge,
    params,
    performAction,
    ratings,
    rateRequest,
    updateDescription,
    viewerId,
  } = props
  const { request } = detail
  const serverDescription = request.description_markdown
  const scope = repoResourceScope(
    live.repo,
    viewerId === 'anonymous' ? null : viewerId,
  )
  const history = useRequestActivityHistory({
    identity: request.permissions.can_view_activity
      ? requestActivityIdentity(scope, request.id)
      : null,
    initialValue: initialActivity,
    load: loadActivity,
    version: String(request.activity_version),
  })
  const checks = useRequestChecks({
    approve: approveChecks,
    identity: requestChecksIdentity(scope, request.id),
    load: loadChecks,
  })
  const autoMerge = useRequestAutoMerge({
    authorize: authorizeAutoMerge,
    cancel: cancelAutoMerge,
    identity: requestAutoMergeIdentity(scope, request.id),
    load: loadAutoMerge,
  })
  const activity = history.activity ?? initialActivity
  const pushes = useMemo(
    () => requestRevisionPushes(activity?.events ?? []),
    [activity],
  )
  const liveRequest = withCurrentMergeability(request, checks.checks)
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
  const paneRef = useRef<HTMLDivElement>(null)
  const moreActions = useRef<HTMLButtonElement>(null)
  const detailsButton = useRef<HTMLButtonElement>(null)
  const [detailsOpen, setDetailsOpen] = useState(false)
  const [descriptionActions, setDescriptionActions] = useState<HTMLElement | null>(null)
  const rail = useDetailPaneRail(paneRef)
  // The rail takes over the drawer's content; narrowing again must not reopen it.
  if (rail && detailsOpen) setDetailsOpen(false)
  const [lifecycleBar, setLifecycleBar] = useState<HTMLDivElement | null>(null)
  const actionClearance = useElementHeight(lifecycleBar)
  const canClaim = workspace?.selected?.attention.reason === 'unclaimed' &&
    workspace.selected.attention.can_claim
  const canRelease = workspace?.selected?.attention.can_release ?? false

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
      <div
        className="request-detail-pane scope-content-enter w-full"
        ref={paneRef}
        style={{ '--request-action-clearance': `${actionClearance}px` } as CSSProperties}
      >
        <RequestDetailHeader
          actions={
            <>
              <Button asChild size="sm" variant="secondary">
                <Link
                  params={{ ...params, requestId: request.id }}
                  to="/$owner/$repo/requests/$requestId/changes"
                >
                  <GitCommit />
                  Changes
                </Link>
              </Button>
              {rail ? null : (
                <Button
                  onClick={() => setDetailsOpen(true)}
                  ref={detailsButton}
                  size="sm"
                  type="button"
                  variant="secondary"
                >
                  <SlidersHorizontal />
                  Details
                </Button>
              )}
              {canClaim ? (
                <Button onClick={workspace?.claim} size="sm" type="button" variant="secondary">
                  <UserRound />
                  I’ll take this
                </Button>
              ) : null}
              {checks.checks?.can_approve ? (
                <Button
                  disabled={checks.approving}
                  onClick={() => void checks.approve()}
                  size="sm"
                  type="button"
                  variant="secondary"
                >
                  <CirclePlay />
                  Approve checks
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
                autoMerge={autoMerge}
                className="fixed inset-x-0 bottom-0 z-30 justify-end border-t border-border bg-background px-3 py-3 pb-[max(0.75rem,env(safe-area-inset-bottom))] min-[701px]:static min-[701px]:border-0 min-[701px]:bg-transparent min-[701px]:p-0"
                ref={setLifecycleBar}
                request={liveRequest}
                viewerId={viewerId}
              />
              <RequestMoreMenu
                actions={requestActions}
                disabled={requestActions.pending !== null || autoMerge.pending !== null}
                onViewActivity={history.openHistory}
                request={request}
                triggerRef={moreActions}
              />
            </>
          }
          request={liveRequest}
        />
        <div className="request-detail-actions px-5 py-2.5 min-[701px]:hidden">
          <Button asChild size="icon-sm" variant="secondary">
            <Link
              aria-label="Back to requests"
              params={params}
              to="/$owner/$repo/requests"
            >
              <ArrowLeft />
            </Link>
          </Button>
        </div>
        {requestActions.error || autoMerge.error ? (
          <p
            className="border-b border-border px-5 py-2 text-sm text-danger-strong sm:px-6 lg:px-8"
            role="alert"
          >
            {requestActions.error ?? autoMerge.error}
          </p>
        ) : null}
        <RequestChecksSection
          checks={checks.checks}
          error={checks.error}
          params={params}
        />
        <RequestDetailsProvider value={{
          actions: requestActions,
          onRate: rateRequest,
          params: requestParams,
          placement: rail ? 'rail' : 'drawer',
          ratings,
          request,
        }}>
          <div className={cn(rail && 'grid grid-cols-[minmax(0,1fr)_300px]')}>
            <div
              className="request-detail-document pt-4"
              data-state={request.state}
            >
              <RequestDescription
                actionsSlot={descriptionActions}
                canEdit={request.permissions.can_edit_identity}
                description={description}
                onSave={saveDescription}
              />
              <div className="request-discussion-heading flex min-h-11 items-center gap-2 border-b border-border px-5 lg:px-7">
                <h2 className="flex items-center gap-2 text-sm font-medium">
                  <MessageSquare className="size-3.5" />
                  Discussion
                </h2>
                <div className="ml-auto flex items-center" ref={setDescriptionActions} />
              </div>
              <RequestRevisionPushesProvider value={pushes}>
                <div className="min-w-0">{children}</div>
              </RequestRevisionPushesProvider>
            </div>
            {rail ? (
              <aside className="min-w-0 border-l border-border">
                <RequestDetails placement="rail" />
              </aside>
            ) : null}
          </div>
          <RequestSideDrawer
            description="Lifecycle, invitees, ratings and git state."
            icon={<SlidersHorizontal />}
            onOpenChange={setDetailsOpen}
            open={detailsOpen}
            returnFocus={detailsButton}
            title="Details"
          >
            <RequestDetails placement="drawer" />
          </RequestSideDrawer>
        </RequestDetailsProvider>

        <RequestActivityDrawer
          activity={history.activity}
          error={history.error}
          load={history.retry}
          loading={history.loading}
          onOpenChange={history.onOpenChange}
          open={history.open}
          params={{ ...params, requestId: request.id }}
          returnFocus={moreActions}
        />
      </div>
    </RequestAttachmentProvider>
  )
}
