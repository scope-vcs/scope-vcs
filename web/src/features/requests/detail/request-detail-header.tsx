import type { RepoParams } from '@/api/types'
import type { RequestChecksResponse, RequestDetailResponse } from '@/api/types.generated'
import { RelativeTimestamp } from '@/components/timestamp'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { Link } from '@tanstack/react-router'
import { Globe } from 'lucide-react'
import { useState, type ReactNode } from 'react'
import { requestChecksSummary, requestFailedCheck } from '../checks/request-check-rows'
import { actorHandle } from './request-actor'
import { authorRecordSummary } from './request-author-record-model'
import { FACT_LINK_CLASS, LinkedRequestLink } from './request-fact-link'
import { RequestAuthorRecordSummary } from './request-author-record'
import {
  REQUEST_LINK_WORDS,
  requestCloseReasonLabel,
  requestMergeabilityLabel,
  requestPublicChecksNote,
  requestStateMark,
} from './request-labels'
import { RequestStateMark } from './request-state-mark'

const DETAIL_CLASS = 'text-sm leading-6 text-muted-foreground'

export function RequestDetailHeader({
  actions,
  backLink,
  checks,
  detail: { asker, author, author_record: authorRecord, claimer, links, request },
  params,
  stateActions,
}: {
  actions: ReactNode
  backLink: ReactNode
  checks: RequestChecksResponse
  detail: RequestDetailResponse
  params: RepoParams
  stateActions: ReactNode
}) {
  const publicNote = request.permissions.can_read_change ? requestPublicChecksNote(checks) : null
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
              ? request.close_reason
                ? (
                  <span className={`${DETAIL_CLASS} min-w-0 [overflow-wrap:anywhere]`}>
                    {request.close_reason === 'duplicate' && links.close_named
                      ? null
                      : <>{requestCloseReasonLabel(request.close_reason)}{' '}</>}
                    <RelativeTimestamp value={settledAt} />
                    {links.close_named ? (
                      <>
                        , {request.close_reason === 'duplicate' ? REQUEST_LINK_WORDS.duplicateOf : REQUEST_LINK_WORDS.fixedBy}{' '}
                        <LinkedRequestLink params={params} request={links.close_named} />
                      </>
                    ) : null}
                  </span>
                )
                : <RelativeTimestamp className={DETAIL_CLASS} value={settledAt} />
              : request.mergeability.status === 'ChecksFailed'
                ? <FailedCheckLink checks={checks} params={params} />
                : null}
          </span>
        </p>
        <div className="request-detail-header-actions">{stateActions}</div>
      </div>
      <div className="request-detail-header-facts mt-2">
        <div className="request-detail-header-meta min-w-0 text-xs leading-5 text-muted-foreground">
          {request.ask ? (
            <>
              <span className="flex flex-wrap items-baseline gap-x-1.5">
                <span className="font-medium text-foreground">{asker?.login}</span>
                <span>opened</span>
                <RelativeTimestamp value={request.ask.asked_at_unix} />
              </span>
              <span>
                from{' '}
                <a className={FACT_LINK_CLASS} href={request.ask.source_url} rel="noreferrer" target="_blank">
                  GitHub #{request.ask.github_issue_number}
                </a>
              </span>
              {author ? <span>working on it: <span className="text-foreground">{author.handle}</span></span> : null}
              {claimer ? <span>reviewing: <span className="text-foreground">{claimer.handle}</span></span> : null}
            </>
          ) : (
            <span className="flex flex-wrap items-baseline gap-x-1.5">
              <span className="font-medium text-foreground">{actorHandle(author)}</span>
              {authorRecord ? <RequestAuthorRecordSummary handle={actorHandle(author)} summary={authorRecordSummary(authorRecord)} /> : null}
              <span>opened</span>
              <RelativeTimestamp
                value={request.submitted_at_unix ?? request.created_at_unix}
              />
              {claimer ? (
                <>
                  <span aria-hidden="true">·</span>
                  <span>reviewing:</span>
                  <span className="text-foreground">{claimer.handle}</span>
                </>
              ) : null}
            </span>
          )}
          {links.resolves.map((resolved) => (
            <span className="flex min-w-0 max-w-full gap-x-1.5" key={resolved.id}>
              {REQUEST_LINK_WORDS.resolves}
              <LinkedRequestLink className="request-detail-header-branch truncate" params={params} request={resolved} />
            </span>
          ))}
          {request.branch_name ? (
            <span className="request-detail-header-branch truncate font-mono" title={request.branch_name}>{request.branch_name}</span>
          ) : null}
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
          className={FACT_LINK_CLASS}
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
