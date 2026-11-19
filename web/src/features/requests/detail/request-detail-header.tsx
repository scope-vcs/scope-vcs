import type { RepoParams } from '@/api/types'
import type { RequestChecksResponse, RequestSummaryResponse } from '@/api/types.generated'
import { RelativeTimestamp } from '@/components/timestamp'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { Link } from '@tanstack/react-router'
import { Globe } from 'lucide-react'
import { useState, type ReactNode } from 'react'
import { requestChecksSummary, requestFailedCheck } from '../checks/request-check-rows'
import { useRequestWorkspace } from '../workspace/request-workspace-context'
import { actorHandle } from './request-actor'
import { requestMergeabilityLabel, requestPublicChecksNote, requestStateMark } from './request-labels'
import { RequestStateMark } from './request-state-mark'

const DETAIL_CLASS = 'text-sm leading-6 text-muted-foreground'

export function RequestDetailHeader({
  actions,
  backLink,
  checks,
  params,
  request,
  stateActions,
}: {
  actions: ReactNode
  backLink: ReactNode
  checks: RequestChecksResponse
  params: RepoParams
  request: RequestSummaryResponse
  stateActions: ReactNode
}) {
  const workspace = useRequestWorkspace()
  const queueItem = workspace?.selected ?? null
  const publicNote = requestPublicChecksNote(checks)
  const settledAt = request.merged_at_unix ?? request.closed_at_unix

  return (
    <header className="request-detail-header px-(--app-inset) pb-2 pt-3 min-[701px]:pt-6">
      <nav aria-label="Request navigation" className="mb-3 min-[701px]:hidden">{backLink}</nav>
      <h1 className="min-w-0 break-words text-[28px] font-semibold leading-[1.15] tracking-[-0.025em] sm:text-[30px]">
        {request.title}
      </h1>
      <div className="request-detail-header-state mt-4">
        <p className="flex min-w-0 items-center gap-2">
          <RequestStateMark mark={requestStateMark(request)} />
          <span className="flex min-w-0 flex-wrap items-baseline gap-x-2.5">
            <span className="text-[17px] font-semibold leading-6 tracking-[-0.012em]">
              {requestMergeabilityLabel(request)}
            </span>
            {settledAt !== null
              ? <RelativeTimestamp className={DETAIL_CLASS} value={settledAt} />
              : request.mergeability.status === 'ChecksFailed'
                ? <FailedCheckLink checks={checks} params={params} />
                : null}
          </span>
        </p>
        <div className="request-detail-header-actions">{stateActions}</div>
      </div>
      <div className="request-detail-header-facts mt-2">
        <div className="request-detail-header-meta min-w-0 text-xs leading-5 text-muted-foreground">
          {queueItem ? (
            <span className="flex flex-wrap items-baseline gap-x-1.5">
              <span className="font-medium text-foreground">{actorHandle(queueItem.author)}</span>
              <span>opened</span>
              <RelativeTimestamp
                value={request.submitted_at_unix ?? request.created_at_unix}
              />
              {queueItem.claimer ? (
                <>
                  <span aria-hidden="true">·</span>
                  <span>reviewing:</span>
                  <span className="text-foreground">{queueItem.claimer.handle}</span>
                </>
              ) : null}
            </span>
          ) : null}
          <span className="request-detail-header-branch truncate font-mono" title={request.name}>{request.name}</span>
          {publicNote ? <PublicChecksLabel note={publicNote} /> : null}
        </div>
        <div className="request-detail-header-actions">{actions}</div>
      </div>
    </header>
  )
}

function PublicChecksLabel({ note }: { note: string }) {
  const [open, setOpen] = useState(false)
  return (
    <TooltipProvider delayDuration={0}>
      <Tooltip onOpenChange={setOpen} open={open}>
        <TooltipTrigger asChild onClick={(event) => {
          event.preventDefault()
          setOpen(true)
        }}>
          <button
            className="inline-flex items-center gap-1 rounded-sm focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
            type="button"
          >
            <Globe aria-hidden="true" className="size-3.5" />
            Public
          </button>
        </TooltipTrigger>
        <TooltipContent>{note}</TooltipContent>
      </Tooltip>
    </TooltipProvider>
  )
}

function FailedCheckLink({ checks, params }: { checks: RequestChecksResponse; params: RepoParams }) {
  const failed = requestFailedCheck(requestChecksSummary(checks))
  if (!failed) return null
  const { check, others } = failed
  return (
    <span className={`${DETAIL_CLASS} [overflow-wrap:anywhere]`}>
      {check.run ? (
        <Link
          className="rounded-sm underline decoration-border-strong underline-offset-[3px] hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
          hash={check.run.hash}
          params={{ ...params, runId: check.run.id }}
          to="/$owner/$repo/runs/$runId"
        >
          {check.name}
        </Link>
      ) : check.name}
      {others ? ` and ${others} more` : null}
    </span>
  )
}
