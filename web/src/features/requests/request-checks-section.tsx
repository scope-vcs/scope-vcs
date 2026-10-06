import type { RepoParams } from '@/api/types'
import type { RequestChecksResponse } from '@/api/types.generated'
import { Link } from '@tanstack/react-router'
import { RunStatusIcon } from '../runs/run-status-icon'
import { type RequestCheckRow, requestCheckRow } from './request-check-rows'
import {
  requestCheckEvaluationNote,
  requestChecksWorkflowWarning,
  requestGitHubPushNote,
  requestPublicGitHubNote,
} from './request-labels'
import { CHECKS_SECTION_CLASS, RequestChecksPending } from './request-checks-pending'

const LOGS_LINK_CLASS =
  'font-mono text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline'

export function RequestChecksSection({
  checks,
  error,
  params,
  requestViewName,
}: {
  checks: RequestChecksResponse | null
  error: string | null
  params: RepoParams
  requestViewName: string
}) {
  if (!checks && !error) return <RequestChecksPending />
  const note = checks ? requestCheckEvaluationNote(checks) : null
  const warning = checks ? requestChecksWorkflowWarning(checks) : null
  const push = requestGitHubPushNote(checks?.github_push ?? null)
  const publicOnGitHub = checks ? requestPublicGitHubNote(checks, requestViewName) : null

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
      {publicOnGitHub ? (
        <p className="mt-2 text-[13px] text-muted-foreground">{publicOnGitHub}</p>
      ) : null}
      {push ? (
        <p
          className={`mt-2 break-words text-[13px] ${push.failed ? 'text-danger-strong' : 'text-muted-foreground'}`}
        >
          {push.text}
        </p>
      ) : null}
      {checks?.checks.length ? (
        <ul className="mt-2.5 grid gap-1.5">
          {checks.checks.map(requestCheckRow).map((row) => (
            <li
              className="flex min-w-0 items-center gap-2 text-[13px]"
              key={row.key}
            >
              <RunStatusIcon state={row.state} />
              <span className="min-w-0 flex-1 truncate">{row.name}</span>
              <span className="shrink-0 text-xs text-muted-foreground">{row.provider}</span>
              <CheckLogs params={params} row={row} />
            </li>
          ))}
        </ul>
      ) : null}
    </section>
  )
}

function CheckLogs({ params, row }: { params: RepoParams; row: RequestCheckRow }) {
  if (!row.logs) {
    return <span className="shrink-0 font-mono text-xs text-muted-foreground">{row.label}</span>
  }
  if ('runId' in row.logs) {
    return (
      <Link
        className={`shrink-0 ${LOGS_LINK_CLASS}`}
        params={{ ...params, runId: row.logs.runId }}
        to="/$owner/$repo/runs/$runId"
      >
        {row.label}
      </Link>
    )
  }
  return (
    <a
      className={`shrink-0 ${LOGS_LINK_CLASS}`}
      href={row.logs.href}
      rel="noopener noreferrer"
      target="_blank"
    >
      {row.label}
    </a>
  )
}
