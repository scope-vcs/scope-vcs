import type { RunActionInput } from '@/api/types'
import type { RepositoryRunDetailResponse } from '@/api/types.generated'
import { SeparatedMeta, SeparatedMetaItem } from '@/components/separated-meta'
import { RelativeTimestamp } from '@/components/timestamp'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { LoaderCircle, RotateCcw, Square } from 'lucide-react'
import type { ReactNode } from 'react'
import { runCanChange } from './repository-run-detail-model'
import { runDisplayState, runTriggerLabel } from './run-formatting'
import { RunDuration } from './run-duration'
import { runJobTallies } from './run-job-tree-model'
import { RUN_TONE_TEXT_CLASS, RunStatusIcon } from './run-status-icon'
import { runDurationLead, runStatus } from './run-status'

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

  return (
    <RunPageHeader
      action={runCanChange(run.state) ? (
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
      error={metadataError}
      jobStates={detail.jobs.map(({ job }) => job.state)}
      name={run.workflow_name}
      onRetry={onRefresh}
      params={params}
      state={state}
    >
      <SeparatedMetaItem>
        <RunHeaderStatus state={state}>
          {runDurationLead(state)}{' '}
          <RunDuration end={run.completed_at_unix} start={run.created_at_unix} />
        </RunHeaderStatus>
      </SeparatedMetaItem>
      <SeparatedMetaItem>
        <code>{run.git_oid.slice(0, 7)}</code>
      </SeparatedMetaItem>
      <SeparatedMetaItem>{runTriggerLabel(run.trigger)}</SeparatedMetaItem>
      <SeparatedMetaItem>
        started <RelativeTimestamp value={run.created_at_unix} />
      </SeparatedMetaItem>
    </RunPageHeader>
  )
}

export function RunPageHeader({
  action,
  children,
  error,
  jobStates,
  name,
  onRetry,
  params,
  state,
}: {
  action: ReactNode
  children: ReactNode
  error: string | null
  jobStates: readonly string[]
  name: string
  onRetry: () => void
  params: RunActionInput
  state: string
}) {
  const tallies = runJobTallies(jobStates.map((jobState) => ({ state: jobState })))
  return (
    <header className="flex flex-wrap items-center gap-x-4 gap-y-2 px-4 py-3">
      <div className="min-w-0 flex-[1_1_20rem]">
        <p className="text-xs text-muted-foreground">
          <Link
            className="hover:text-foreground hover:underline hover:underline-offset-2"
            params={{ owner: params.owner, repo: params.repo }}
            to="/$owner/$repo/runs"
          >
            Runs
          </Link>
          <span aria-hidden="true" className="px-1 text-muted-foreground/60">/</span>
          <span>{name}</span>
        </p>
        <div className="mt-0.5 flex min-w-0 items-center gap-2">
          <RunStatusIcon state={state} />
          <h1 className="min-w-0 break-words text-base font-semibold leading-6 tracking-[-0.01em]">{name}</h1>
        </div>
        <SeparatedMeta className="mt-0.5 text-xs text-muted-foreground">{children}</SeparatedMeta>
      </div>
      {tallies.length > 0 || action ? (
        <div className="flex flex-wrap items-center gap-2">
          {tallies.length > 0 ? (
            <ul aria-label="Jobs by status" className="flex flex-wrap items-center gap-1.5">
              {tallies.map((tally) => (
                <li
                  className={cn(
                    'inline-flex h-6 items-center rounded-full border px-2.5 text-xs font-medium tabular-nums',
                    tally.tone === 'danger'
                      ? 'border-danger-border bg-danger-soft text-danger-strong'
                      : 'border-border bg-card text-foreground',
                  )}
                  key={tally.tone}
                >
                  {tally.count} {tally.label}
                </li>
              ))}
            </ul>
          ) : null}
          {action}
        </div>
      ) : null}
      <LiveUpdatesPaused error={error} onRetry={onRetry} />
    </header>
  )
}

export function RunHeaderStatus({ children, state }: { children: ReactNode; state: string }) {
  return (
    <span className={cn('font-medium', RUN_TONE_TEXT_CLASS[runStatus(state).tone])} suppressHydrationWarning>
      {children}
    </span>
  )
}

function LiveUpdatesPaused({ error, onRetry }: { error: string | null; onRetry: () => void }) {
  if (!error) return null
  return (
    <p
      className="flex basis-full flex-wrap items-center gap-2 text-xs text-muted-foreground"
      role="status"
    >
      <span>Live updates paused. {error}</span>
      <button
        className="underline underline-offset-2 hover:text-foreground"
        onClick={onRetry}
        type="button"
      >
        Retry now
      </button>
    </p>
  )
}
