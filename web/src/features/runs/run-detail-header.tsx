import type { RunActionInput } from '@/api/types'
import type { RepositoryRunDetailResponse } from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { formatUnixDate, formatUnixDateUtc } from '@/lib/date-format'
import { useHydrated } from '@/lib/use-hydrated'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { LoaderCircle, RotateCcw, Square } from 'lucide-react'
import { runCanChange } from './repository-run-detail-model'
import { runDisplayState, runTriggerLabel } from './run-formatting'
import { RunDuration } from './run-duration'
import { RUN_TONE_TEXT_CLASS, RunStatusIcon } from './run-status-icon'
import { runDurationLead, runStatus } from './run-status'

/** Two quiet lines: where you are, then how the run went ("Running for 12m
 * 41s", its trigger and commit). One action sits beside them: Cancel while
 * the run can still change, Run again once it has finished. */
export function RunDetailHeader({
  detail,
  metadataError,
  onCancel,
  onRefresh,
  onRetry,
  params,
  pendingAction,
}: {
  detail: RepositoryRunDetailResponse
  metadataError: string | null
  onCancel: () => void
  onRefresh: () => void
  onRetry: () => void
  params: RunActionInput
  pendingAction: 'cancel' | 'retry' | null
}) {
  const { run } = detail
  const state = runDisplayState(run)
  const hydrated = useHydrated()
  const formatDate = hydrated ? formatUnixDate : formatUnixDateUtc

  return (
    <header className="px-4 py-3">
      <div className="flex items-center gap-4">
        <div className="min-w-0 flex-1">
          <h1 className="flex min-w-0 items-baseline gap-1.5 text-[15px] leading-[22px]">
            <Link
              className="text-muted-foreground hover:text-foreground"
              params={{ owner: params.owner, repo: params.repo }}
              to="/$owner/$repo/runs"
            >
              Runs
            </Link>
            <span aria-hidden="true" className="text-muted-foreground/50">/</span>
            <span className="truncate font-semibold tracking-[-0.01em]">{run.workflow_name}</span>
          </h1>
          <p className="mt-0.5 flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs text-muted-foreground">
            <span
              className={cn('inline-flex items-center gap-1.5', RUN_TONE_TEXT_CLASS[runStatus(state).tone])}
              suppressHydrationWarning
              title={`Started ${formatDate(run.created_at_unix)} · updated ${formatDate(run.updated_at_unix)}`}
            >
              <RunStatusIcon state={state} />
              <span>
                {runDurationLead(state)}{' '}
                <RunDuration end={run.completed_at_unix} start={run.created_at_unix} />
              </span>
            </span>
            <span className="whitespace-nowrap">
              <span aria-hidden="true" className="mr-1.5 opacity-50">·</span>
              {runTriggerLabel(run.trigger)}
            </span>
            <span className="whitespace-nowrap">
              <span aria-hidden="true" className="mr-1.5 opacity-50">·</span>
              <code>{run.git_oid.slice(0, 7)}</code>
            </span>
          </p>
        </div>
        {runCanChange(run.state) ? (
          <Button
            disabled={!run.can_cancel || pendingAction !== null}
            onClick={onCancel}
            size="sm"
            variant="secondary"
          >
            {pendingAction === 'cancel' ? <LoaderCircle className="animate-spin" /> : <Square />}
            Cancel
          </Button>
        ) : (
          <Button
            disabled={!run.can_retry || pendingAction !== null}
            onClick={onRetry}
            size="sm"
            variant="secondary"
          >
            {pendingAction === 'retry' ? <LoaderCircle className="animate-spin" /> : <RotateCcw />}
            Run again
          </Button>
        )}
      </div>
      {metadataError ? (
        <p
          className="mt-2 flex flex-wrap items-center gap-2 text-xs text-muted-foreground"
          role="status"
        >
          <span>Live updates paused. {metadataError}</span>
          <button
            className="underline underline-offset-2 hover:text-foreground"
            onClick={onRefresh}
            type="button"
          >
            Retry now
          </button>
        </p>
      ) : null}
    </header>
  )
}
