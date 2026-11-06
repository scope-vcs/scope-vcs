import type { RepoParams } from '@/api/types'
import type { RequestChecksResponse } from '@/api/types.generated'
import { cn } from '@/lib/utils'
import { CheckCircle2, ChevronDown, Circle, Globe, LoaderCircle, MinusCircle, XCircle } from 'lucide-react'
import { useId, useState } from 'react'
import { RUN_TONE_TEXT_CLASS } from '../runs/run-status-icon'
import { requestChecksSummary, requestPullRequestView, requestReviewBotRows } from './request-check-rows'
import { RequestCheckWorkflows } from './request-check-workflows'
import {
  requestCheckEvaluationNote,
  requestChecksWorkflowWarning,
  requestPublicChecksNote,
} from './request-labels'
import { CHECKS_SECTION_CLASS } from './request-checks-pending'

export function RequestChecksSection({
  checks,
  error,
  params,
}: {
  checks: RequestChecksResponse
  error: string | null
  params: RepoParams
}) {
  const [expanded, setExpanded] = useState(false)
  const detailsId = useId()
  const listId = useId()
  const pullRequest = requestPullRequestView(checks.github_pull_request)
  const reviewBots = requestReviewBotRows(checks.review_bots)
  if (checks.state === 'no-checks' && !pullRequest && !reviewBots.length) return null

  const note = requestCheckEvaluationNote(checks)
  const warning = requestChecksWorkflowWarning(checks)
  const publicNote = requestPublicChecksNote(checks)
  const summary = requestChecksSummary(checks)
  const passed = summary.status?.tone === 'success'
  const StatusIcon = summary.status?.tone === 'danger' ? XCircle
    : summary.status?.tone === 'running' ? LoaderCircle
      : passed ? CheckCircle2
        : summary.status?.tone === 'waiting' ? Circle : MinusCircle
  const reviewSummary = reviewBots.length
    ? checks.review_bots.every((bot) => bot.state === 'clear')
      ? 'Review clear'
      : [...new Set(reviewBots.filter((bot) => bot.tone !== 'success').map((bot) => bot.label))].join(' · ')
    : null
  const detailsAvailable = summary.all.length > summary.attention.length || reviewBots.length > 0 || !!publicNote

  return (
    <section
      aria-label="CI"
      className={CHECKS_SECTION_CLASS}
    >
      <div className="grid grid-cols-[16px_minmax(0,1fr)_auto] items-center gap-x-1.5 text-xs leading-5">
        <StatusIcon aria-hidden="true" className={cn('size-4', RUN_TONE_TEXT_CLASS[summary.status?.tone ?? 'inert'])} />
        <h2 className="text-[13px] font-semibold">
          {passed ? 'Checks passed' : summary.status ? `CI · ${summary.status.text}` : 'CI'}
        </h2>
        <div className="flex flex-wrap items-center justify-end gap-x-2.5 text-muted-foreground">
          {publicNote ? (
            <span aria-label="Changes are public" className="inline-flex items-center gap-1" title={publicNote}>
              <Globe aria-hidden="true" className="size-3.5" />
              Public
            </span>
          ) : null}
          {pullRequest?.url ? (
            <a aria-label={pullRequest.text} className="hover:text-foreground hover:underline underline-offset-4" href={pullRequest.url} rel="noreferrer" target="_blank">
              PR #{checks.github_pull_request?.number}
            </a>
          ) : null}
        </div>
        <div className="col-start-2 flex flex-wrap gap-x-1 text-muted-foreground">
          {summary.all.length ? <span>{summary.all.length} {summary.all.length === 1 ? 'check' : 'checks'}</span> : null}
          {summary.counts ? <span>· {summary.counts}</span> : null}
          {reviewSummary ? <span>{summary.all.length ? '· ' : ''}{reviewSummary}</span> : null}
        </div>
        {detailsAvailable ? (
          <button
            aria-controls={detailsId}
            aria-expanded={expanded}
            className="col-start-3 flex items-center justify-self-end gap-1 rounded-sm text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
            onClick={() => setExpanded((open) => !open)}
            type="button"
          >
            Details
            <ChevronDown aria-hidden="true" className={cn('size-3 transition-transform', expanded && 'rotate-180')} />
          </button>
        ) : null}
      </div>
      {error ? (
        <p className="mt-2 text-[13px] text-danger-strong" role="alert">
          {error}
        </p>
      ) : null}
      {note ? <p className="mt-2 text-[13px] text-muted-foreground">{note}</p> : null}
      {warning ? (
        <p className="mt-2 text-[13px] text-warning-strong" role="note">
          {warning}
        </p>
      ) : null}
      {summary.startError ? (
        <p
          className={cn(
            'mt-1 break-words text-[13px]',
            summary.startError.failed ? 'text-danger-strong' : 'text-muted-foreground',
          )}
        >
          {summary.startError.text}
        </p>
      ) : null}
      {pullRequest && (!pullRequest.url || checks.github_pull_request?.state !== 'open') ? (
        <p className="mt-1 text-xs text-muted-foreground">{pullRequest.text}</p>
      ) : null}
      {pullRequest?.note ? (
        <p className={cn('mt-1 break-words text-xs', pullRequest.note.failed ? 'text-danger-strong' : 'text-muted-foreground')}>
          {pullRequest.note.text}
        </p>
      ) : null}
      <div id={detailsId}>
        <RequestCheckWorkflows id={listId} params={params} rows={expanded ? summary.all : summary.attention} />
        <div hidden={!expanded}>
          {reviewBots.length ? (
            <>
              <p className="mt-2 text-xs text-muted-foreground">Review bots</p>
              <ul aria-label="Review bots" className="mt-0.5 space-y-0.5 text-xs">
                {reviewBots.map((bot) => (
                  <li className="flex flex-wrap items-baseline gap-x-2" key={bot.login}>
                    <span className="min-w-0 break-words font-mono [overflow-wrap:anywhere]">{bot.login}</span>
                    {bot.required ? <span className="text-xs text-muted-foreground">required</span> : null}
                    <span className={cn('ml-auto text-xs', RUN_TONE_TEXT_CLASS[bot.tone])}>{bot.label}</span>
                  </li>
                ))}
              </ul>
            </>
          ) : null}
          {publicNote ? <p className="mt-2 text-xs text-muted-foreground">{publicNote}</p> : null}
        </div>
      </div>
    </section>
  )
}
