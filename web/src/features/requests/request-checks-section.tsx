import type { RepoParams } from '@/api/types'
import type { RequestChecksResponse } from '@/api/types.generated'
import { cn } from '@/lib/utils'
import { ChevronDown, Globe } from 'lucide-react'
import { useId, useState } from 'react'
import { RunStatusIcon } from '../runs/run-status-icon'
import { requestChecksSummary } from './request-check-rows'
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
  requestViewName,
}: {
  checks: RequestChecksResponse
  error: string | null
  params: RepoParams
  requestViewName: string
}) {
  const [expanded, setExpanded] = useState(false)
  const listId = useId()
  if (checks.state === 'no-checks') return null

  const note = requestCheckEvaluationNote(checks)
  const warning = requestChecksWorkflowWarning(checks)
  const publicNote = requestPublicChecksNote(checks, requestViewName)
  const summary = requestChecksSummary(checks)
  const folded = summary.all.length - summary.attention.length

  return (
    <section
      aria-label="CI"
      className={CHECKS_SECTION_CLASS}
    >
      <h2 className="label-mono text-muted-foreground">CI</h2>
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
      {summary.lead ? (
        <p className="mt-2 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[13px]">
          <span className="inline-flex items-center gap-1.5 font-medium">
            <RunStatusIcon state={summary.lead.state} />
            {summary.lead.text}
          </span>
          {summary.counts ? (
            <span className="font-mono text-xs text-muted-foreground">· {summary.counts}</span>
          ) : null}
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
      {publicNote ? (
        <p className="mt-1.5 flex items-start gap-1.5 text-xs text-warning-strong">
          <Globe aria-hidden="true" className="mt-px size-3.5 shrink-0" />
          {publicNote}
        </p>
      ) : null}
      <RequestCheckWorkflows id={listId} params={params} rows={expanded ? summary.all : summary.attention} />
      {folded ? (
        <button
          aria-controls={expanded ? listId : undefined}
          aria-expanded={expanded}
          className="mt-1.5 flex items-center gap-1 font-mono text-xs text-muted-foreground hover:text-foreground"
          onClick={() => setExpanded((open) => !open)}
          type="button"
        >
          {expanded ? 'Show less' : `Show all ${summary.all.length} results`}
          <ChevronDown aria-hidden="true" className={cn('size-3 transition-transform', expanded && 'rotate-180')} />
        </button>
      ) : null}
    </section>
  )
}
