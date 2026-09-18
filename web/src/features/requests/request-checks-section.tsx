import type { RepoParams } from '@/api/types'
import type {
  RequestCheckResponse,
  RequestChecksResponse,
} from '@/api/types.generated'
import { Link } from '@tanstack/react-router'
import { runStatus } from '../runs/run-status'
import { RunStatusIcon } from '../runs/run-status-icon'
import { requestCheckEvaluationNote } from './request-labels'

/** What the request head owes before it can merge: one row per workflow. */
export function RequestChecksSection({
  checks,
  error,
  params,
}: {
  checks: RequestChecksResponse | null
  error: string | null
  params: RepoParams
}) {
  if (!checks && !error) return null
  const note = checks ? requestCheckEvaluationNote(checks) : null

  return (
    <section
      aria-label="Checks"
      className="border-b border-border px-5 py-4 sm:px-6 lg:px-8"
    >
      <h2 className="label-mono text-muted-foreground">checks</h2>
      {error ? (
        <p className="mt-2 text-[13px] text-danger-strong" role="alert">
          {error}
        </p>
      ) : null}
      {note ? <p className="mt-2 text-[13px] text-muted-foreground">{note}</p> : null}
      {checks?.checks.length ? (
        <ul className="mt-2.5 grid gap-1.5">
          {checks.checks.map((check) => (
            <li
              className="flex min-w-0 items-center gap-2 text-[13px]"
              key={check.workflow_path}
            >
              <RunStatusIcon state={checkState(check)} />
              <span className="min-w-0 flex-1 truncate">{check.workflow_name}</span>
              {check.run_id ? (
                <Link
                  className="font-mono text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline"
                  params={{ ...params, runId: check.run_id }}
                  to="/$owner/$repo/runs/$runId"
                >
                  {checkStateLabel(check)}
                </Link>
              ) : (
                <span className="font-mono text-xs text-muted-foreground">
                  {checkStateLabel(check)}
                </span>
              )}
            </li>
          ))}
        </ul>
      ) : null}
    </section>
  )
}

/** A recorded check without a run is waiting, not passing. */
function checkState(check: RequestCheckResponse) {
  return check.run_state ?? 'pending'
}

function checkStateLabel(check: RequestCheckResponse) {
  return check.run_state ? runStatus(check.run_state).label : 'not started'
}
