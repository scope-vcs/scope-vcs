import type { RepoLiveState, RepoParams } from '@/api/types'
import type {
  RequestChecksResponse,
  RequestStateResponse,
  RequestMutationResponse,
  RequestAutoMergeResponse,
  RequestRatingResponse,
  RequestRatingsResponse,
} from '@/api/types.generated'
import type { RateRequestInput } from '@/api/requests'
import { EmptyState } from '@/components/empty-state'
import { PageContent } from '@/components/page-header'
import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import {
  ArrowLeft,
  ShieldQuestion,
  UserRound,
  UserRoundMinus,
} from 'lucide-react'
import { type ReactNode, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { mainScrollContainer } from '@/components/main-content'
import { RequestActivityDrawer } from './request-activity-drawer'
import type {
  RequestActionCommand,
  RequestActionResult,
} from './request-actions-api'
import type {
  AuthorizeRequestAutoMergeInput,
  CancelRequestAutoMergeInput,
} from '../checks/request-auto-merge-api'
import { loadRequestChildrenForViewer } from '@/routes/-request-state-actions'
import { loadMoreRequestChildren } from './request-state-resource'
import { RequestStackContext } from './request-stack-context'
import { RequestChecksSection } from '../checks/request-checks-section'
import { RequestCiApproval } from '../checks/request-ci-approval'
import { RequestChangeCommand } from './request-change-command'
import { RequestDetailHeader } from './request-detail-header'
import { RequestDetailsMenu } from './request-details'
import type { RequestActivityPage } from '../discussion/request-discussion-types'
import { RequestDescription } from './request-description'
import type { UpdateDescriptionInput } from '../discussion/request-discussion-api'
import { RequestLifecycleActions } from './request-lifecycle-actions'
import { RequestMoreMenu } from './request-more-menu'
import { RequestResolvedBy } from './request-resolved-by'
import { useElementHeight } from './use-element-height'
import { useRequestActions } from './use-request-actions'
import { useRequestActivityHistory } from './use-request-activity-history'
import { useRequestChecks } from '../checks/use-request-checks'
import { useRequestAutoMerge } from '../checks/use-request-auto-merge'
import { requestActivityIdentity } from './request-activity-resource'
import { RequestChangesMenu } from '../changes/request-changes-menu'
import {
  RequestAttachmentProvider,
  type RequestAttachmentActions,
} from '../attachments/request-attachment-context'
import { useRequestWorkspace } from '../workspace/request-workspace-context'
import './request-detail.css'
import { Entrance } from '@/components/entrance'

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
  childrenError: string | null
  approveChecks: (expectedHeadOid: string) => Promise<RequestChecksResponse>
  authorizeAutoMerge: (input: Pick<
    AuthorizeRequestAutoMergeInput,
    'expected_head_oid' | 'expected_revision_id'
  >) => Promise<RequestAutoMergeResponse>
  attachmentActions: RequestAttachmentActions
  children: ReactNode
  state: RequestStateResponse
  stateError: string | null
  identity: string
  scope: string
  live: RepoLiveState
  loadActivity: (signal: AbortSignal) => Promise<RequestActivityPage>
  loadRatings: (signal: AbortSignal) => Promise<RequestRatingsResponse>
  params: RepoParams
  performAction: (command: RequestActionCommand) => Promise<RequestActionResult>
  cancelAutoMerge: (input: Pick<
    CancelRequestAutoMergeInput,
    'expected_intent_id'
  >) => Promise<RequestAutoMergeResponse>
  rateRequest: (input: RateRequestInput) => Promise<RequestRatingResponse>
  updateDescription: (input: UpdateDescriptionInput) => Promise<RequestMutationResponse>
  viewerId: string | null
}

export function RequestDetailPage(props: RequestDetailPageProps) {
  const {
    approveChecks,
    authorizeAutoMerge,
    cancelAutoMerge,
    children,
    childrenError,
    attachmentActions,
    state,
    stateError,
    identity,
    scope,
    live,
    loadActivity,
    loadRatings,
    params,
    performAction,
    rateRequest,
    updateDescription,
    viewerId,
  } = props
  const [childrenLoading, setChildrenLoading] = useState(false)
  const loadChildren = async () => {
    if (childrenLoading) return
    setChildrenLoading(true)
    try {
      await loadMoreRequestChildren(identity, (cursor, signal) => loadRequestChildrenForViewer({ ...params, request_id: state.detail.request.id, cursor }, viewerId, signal))
    } finally {
      setChildrenLoading(false)
    }
  }
  const { asker, request } = state.detail
  const { can_attach_change: canAttachChange, can_read_change: canReadChange } = request.permissions
  const serverDescription = request.description_markdown
  const activity = {
    identity: request.permissions.can_view_activity
      ? requestActivityIdentity(scope, request.id)
      : null,
    load: loadActivity,
    version: String(request.activity_version),
  }
  const history = useRequestActivityHistory(activity)
  const checks = useRequestChecks({
    approve: approveChecks,
    identity,
    checks: state.checks,
  })
  const autoMerge = useRequestAutoMerge({
    authorize: authorizeAutoMerge,
    cancel: cancelAutoMerge,
    identity,
    status: state.auto_merge,
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
  const moreActions = useRef<HTMLButtonElement>(null)
  const [lifecycleBar, setLifecycleBar] = useState<HTMLDivElement | null>(null)
  const actionClearance = useElementHeight(lifecycleBar)
  useLayoutEffect(() => {
    const main = mainScrollContainer()
    if (!main) return
    main.style.setProperty('--request-action-clearance', `${actionClearance}px`)
    return () => {
      main.style.removeProperty('--request-action-clearance')
    }
  }, [actionClearance])
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
      viewerId={viewerId ?? 'anonymous'}
    >
      <Entrance className="w-full">
        <RequestDetailHeader
          actions={
            <>
              {canReadChange ? (
                <RequestChangesMenu
                  activity={activity}
                  diffStat={state.detail.diff_stat}
                  params={{ ...params, requestId: request.id }}
                />
              ) : null}
              <RequestDetailsMenu
                actions={requestActions}
                key={identity}
                loadRatings={loadRatings}
                onRate={rateRequest}
                params={requestParams}
                ratingsIdentity={identity}
                request={request}
              />
              {canClaim ? (
                <Button onClick={workspace.claim} size="sm" type="button" variant="secondary">
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
              <RequestMoreMenu
                actions={requestActions}
                disabled={requestActions.pending !== null || autoMerge.pending !== null}
                key={request.id}
                onViewActivity={history.openHistory}
                params={params}
                request={request}
                resolvedBy={state.detail.links.resolved_by}
                triggerRef={moreActions}
              />
            </>
          }
          backLink={(
            <Link
              className="flex w-fit items-center gap-1.5 rounded py-1 text-xs text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
              params={params}
              to="/$owner/$repo/requests"
            >
              <ArrowLeft aria-hidden="true" className="size-3.5" /> Requests
            </Link>
          )}
          checks={checks.checks}
          detail={state.detail}
          params={params}
          stateActions={
            <>
              {canReadChange ? <RequestCiApproval controller={checks} key={identity} /> : null}
              <RequestLifecycleActions
                actions={requestActions}
                autoMerge={autoMerge}
                className="fixed inset-x-0 bottom-0 z-30 justify-end border-t border-border bg-background px-(--app-inset) py-3 pb-[max(0.75rem,env(safe-area-inset-bottom))] min-[701px]:static min-[701px]:border-0 min-[701px]:bg-transparent min-[701px]:p-0"
                ref={setLifecycleBar}
                request={request}
                viewerId={state.viewer?.id ?? 'anonymous'}
              />
            </>
          }
        />
        {requestActions.error || autoMerge.error || stateError ? (
          <p
            className="border-b border-border px-(--app-inset) py-2 text-sm text-danger-strong"
            role="alert"
          >
            {requestActions.error ?? autoMerge.error ?? stateError}
          </p>
        ) : null}
        <RequestResolvedBy params={params} requests={state.detail.links.resolved_by} />
        <RequestStackContext detail={state.detail} params={params} childrenLoading={childrenLoading} childrenError={childrenError} onLoadChildren={() => { void loadChildren() }} />
        {request.ask && canAttachChange ? <RequestChangeCommand ask={request.ask} /> : null}
        {canReadChange ? (
          <RequestChecksSection
            checks={checks.checks}
            error={checks.error}
            params={params}
          />
        ) : null}
        <div className="min-w-0 pt-4">
          <RequestDescription
            ask={request.ask && asker ? { login: asker.login, text: request.ask.text_markdown } : null}
            canEdit={request.permissions.can_edit_identity}
            description={description}
            onSave={saveDescription}
          />
          <div className="min-w-0">{children}</div>
        </div>

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
      </Entrance>
    </RequestAttachmentProvider>
  )
}
