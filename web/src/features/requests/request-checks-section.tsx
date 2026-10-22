import type { RepoParams } from '@/api/types'
import type { RequestChecksResponse } from '@/api/types.generated'
import { cn } from '@/lib/utils'
import { ChevronDown } from 'lucide-react'
import { useId, useState } from 'react'
import { RUN_TONE_TEXT_CLASS } from '../runs/run-status-icon'
import { requestChecksSummary, requestPullRequestView } from './request-check-rows'
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
  const listId = useId()
  const pullRequest = requestPullRequestView(checks.github_pull_request)
  if (checks.state === 'no-checks' && !pullRequest) return null

  const note = requestCheckEvaluationNote(checks)
  const warning = requestChecksWorkflowWarning(checks)
  const publicNote = requestPublicChecksNote(checks)
  const summary = requestChecksSummary(checks)
  const folded = summary.all.length - summary.attention.length

  return (
    <section
      aria-label="CI"
      className={CHECKS_SECTION_CLASS}
    >
      <div className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
        <h2 className="section-label">CI</h2>
        {summary.status ? (
          <span className={cn('text-xs font-medium', RUN_TONE_TEXT_CLASS[summary.status.tone])}>{summary.status.text}</span>
        ) : null}
        {summary.counts ? (
          <span className="text-xs text-muted-foreground">{summary.status ? `· ${summary.counts}` : summary.counts}</span>
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
      {pullRequest ? (
        <div className="mt-1.5 text-[13px]">
          {pullRequest.url ? (
            <a
              className="text-foreground underline-offset-4 hover:underline"
              href={pullRequest.url}
              rel="noreferrer"
              target="_blank"
            >
              {pullRequest.text}
            </a>
          ) : (
            <span className="text-muted-foreground">{pullRequest.text}</span>
          )}
          {pullRequest.note ? (
            <p
              className={cn(
                'mt-0.5 break-words',
                pullRequest.note.failed ? 'text-danger-strong' : 'text-muted-foreground',
              )}
            >
              {pullRequest.note.text}
            </p>
          ) : null}
        </div>
      ) : null}
      <RequestCheckWorkflows id={listId} params={params} rows={expanded ? summary.all : summary.attention} />
      {folded ? (
        <button
          aria-controls={expanded ? listId : undefined}
          aria-expanded={expanded}
          className="mt-1.5 flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
          onClick={() => setExpanded((open) => !open)}
          type="button"
        >
          {expanded ? 'Show less' : `Show all ${summary.all.length} results`}
          <ChevronDown aria-hidden="true" className={cn('size-3 transition-transform', expanded && 'rotate-180')} />
        </button>
      ) : null}
      {publicNote ? <p className="mt-2 text-xs text-muted-foreground">{publicNote}</p> : null}
    </section>
  )
}
