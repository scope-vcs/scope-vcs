import type { RepoParams } from '@/api/types'
import type { RequestChecksResponse } from '@/api/types.generated'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { ChevronDown, ChevronRight, Globe } from 'lucide-react'
import { useId, useState } from 'react'
import { RunStatusIcon } from '../runs/run-status-icon'
import { type RequestCheckRow, requestCheckTree, requestChecksSummary } from './request-check-rows'
import {
  requestCheckEvaluationNote,
  requestChecksWorkflowWarning,
  requestPublicChecksNote,
} from './request-labels'
import { CHECKS_SECTION_CLASS, RequestChecksPending } from './request-checks-pending'

const ROW_CLASS = 'group -mx-1.5 flex min-w-0 items-center gap-2 rounded-md px-1.5 py-1 text-[13px]'
const INDENT_PX = 14

export function RequestChecksSection({
  checks,
  error,
  params,
}: {
  checks: RequestChecksResponse | null
  error: string | null
  params: RepoParams
}) {
  const [expanded, setExpanded] = useState(false)
  const listId = useId()
  if (!checks && !error) return <RequestChecksPending />
  const note = checks ? requestCheckEvaluationNote(checks) : null
  const warning = checks ? requestChecksWorkflowWarning(checks) : null
  const publicNote = checks ? requestPublicChecksNote(checks) : null
  const summary = checks ? requestChecksSummary(checks) : null
  const folded = summary ? summary.all.length - summary.attention.length : 0

  return (
    <section
      aria-label="Checks"
      className={CHECKS_SECTION_CLASS}
    >
      <h2 className="label-mono text-muted-foreground">checks</h2>
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
      {summary?.lead ? (
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
      {summary?.startError ? (
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
      {summary && (expanded ? summary.all.length : summary.attention.length) ? (
        <ul className="mt-2 grid grid-cols-[minmax(0,1fr)]" id={listId}>
          {expanded
            ? requestCheckTree(summary.all).map((line) =>
                line.kind === 'group' ? (
                  <li
                    className="truncate pt-1.5 pb-0.5 text-xs text-muted-foreground"
                    key={line.key}
                    style={{ paddingLeft: line.depth * INDENT_PX }}
                  >
                    {line.name}
                  </li>
                ) : (
                  <li key={line.row.key}>
                    <CheckRow depth={line.depth} params={params} row={line.row} />
                  </li>
                ),
              )
            : summary.attention.map((row) => (
                <li key={row.key}>
                  <CheckRow params={params} row={row} showParent />
                </li>
              ))}
        </ul>
      ) : null}
      {summary && folded ? (
        <button
          aria-controls={expanded ? listId : undefined}
          aria-expanded={expanded}
          className="mt-1.5 flex items-center gap-1 font-mono text-xs text-muted-foreground hover:text-foreground"
          onClick={() => setExpanded((open) => !open)}
          type="button"
        >
          {expanded ? 'Show less' : `Show all ${summary.all.length}`}
          <ChevronDown aria-hidden="true" className={cn('size-3 transition-transform', expanded && 'rotate-180')} />
        </button>
      ) : null}
    </section>
  )
}

function CheckRow({
  depth = 0,
  params,
  row,
  showParent = false,
}: {
  depth?: number
  params: RepoParams
  row: RequestCheckRow
  showParent?: boolean
}) {
  const parent = showParent ? row.parents.at(-1) : undefined
  const content = (
    <>
      <RunStatusIcon state={row.state} />
      <span className={cn('flex min-w-0 flex-1', row.tone === 'inert' && 'text-muted-foreground')} title={row.name}>
        {parent ? (
          <>
            <span className="min-w-0 truncate text-muted-foreground">{parent}</span>
            <span className="shrink-0 text-muted-foreground">{'\u00a0/\u00a0'}</span>
          </>
        ) : null}
        <span className="max-w-full shrink-0 truncate">{row.leaf}</span>
      </span>
      {row.tone === 'success' || row.tone === 'inert' ? null : (
        <span
          className={cn(
            'shrink-0 font-mono text-xs',
            row.tone === 'danger' ? 'text-danger-strong' : 'text-muted-foreground',
          )}
        >
          {row.label}
        </span>
      )}
    </>
  )
  const style = depth ? { paddingLeft: 6 + depth * INDENT_PX } : undefined
  if (!row.runId) {
    return (
      <div className={ROW_CLASS} style={style}>
        {content}
        <span aria-hidden="true" className="size-3 shrink-0" />
      </div>
    )
  }
  return (
    <Link
      className={cn(ROW_CLASS, 'hover:bg-accent focus-visible:bg-accent')}
      params={{ ...params, runId: row.runId }}
      style={style}
      to="/$owner/$repo/runs/$runId"
    >
      {content}
      <ChevronRight
        aria-hidden="true"
        className="size-3 shrink-0 text-muted-foreground opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100"
      />
    </Link>
  )
}
