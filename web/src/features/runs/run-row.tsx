import type { RepoParams } from '@/api/types'
import type { RepositoryRunHistoryPageResponse } from '@/api/types.generated'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { runDisplayState, runTriggerLabel } from './run-formatting'
import { runStatus } from './run-status'
import { RunDuration } from './run-duration'
import { RunStatusIcon } from './run-status-icon'
import { RelativeTimestamp } from '@/components/timestamp'
import {
  RUN_HISTORY_ROW_CLASS,
  RUN_HISTORY_ROW_FIGURE_CLASS,
  RUN_HISTORY_ROW_ICON_CLASS,
  RUN_HISTORY_ROW_PRIMARY_CLASS,
  RUN_ROW_DURATION_CLASS,
  RUN_ROW_TIMESTAMP_CLASS,
} from './run-layout'

export function RunRow({
  params,
  run,
}: {
  params: RepoParams
  run: RepositoryRunHistoryPageResponse['runs'][number]
}) {
  const state = runDisplayState(run)
  const isRunning = runStatus(state).tone === 'running'

  return (
    <Link
      className={cn(
        RUN_HISTORY_ROW_CLASS,
        'group outline-none transition-colors hover:bg-accent/50 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring',
        isRunning && 'bg-info-soft/40',
      )}
      params={{ ...params, runId: run.id }}
      to="/$owner/$repo/runs/$runId"
    >
      <span className={RUN_HISTORY_ROW_ICON_CLASS}>
        <RunStatusIcon state={state} />
      </span>
      <span className={RUN_HISTORY_ROW_PRIMARY_CLASS}>
        <span className="col-span-3 col-start-2 row-start-1 truncate text-sm font-medium">
          {run.workflow_name}
        </span>
        <span className="col-start-2 row-start-2 truncate text-xs text-muted-foreground">
          <span className="font-mono">#{run.git_oid.slice(0, 7)}</span>
          <span className="text-muted-foreground/70">
            {' '}· {runTriggerLabel(run.trigger)}
          </span>
        </span>
      </span>
      <span className={cn(RUN_ROW_DURATION_CLASS, RUN_HISTORY_ROW_FIGURE_CLASS, 'col-start-3')}>
        <RunDuration end={run.completed_at_unix} start={run.created_at_unix} />
      </span>
      <span className={cn(RUN_ROW_TIMESTAMP_CLASS, RUN_HISTORY_ROW_FIGURE_CLASS, 'col-start-4')}>
        <RelativeTimestamp value={run.updated_at_unix} />
      </span>
    </Link>
  )
}
