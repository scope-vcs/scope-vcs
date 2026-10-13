import type { RunActionInput } from '@/api/types'
import type { RepositoryRunDetailResponse } from '@/api/types.generated'
import { Button } from '@/components/ui/button'
import { formatUnixDate, formatUnixDateUtc } from '@/lib/date-format'
import { useHydrated } from '@/lib/use-hydrated'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { LoaderCircle, RotateCcw, Square } from 'lucide-react'
import type { ReactNode } from 'react'
import { runCanChange } from './repository-run-detail-model'
import { runDisplayState, runTriggerLabel } from './run-formatting'
import { RunDuration } from './run-duration'
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
  const hydrated = useHydrated()
  const formatDate = hydrated ? formatUnixDate : formatUnixDateUtc

  return (
    <header className="px-4 py-3">
      <div className="flex items-center gap-4">
        <div className="min-w-0 flex-1">
          <RunHeaderTitle name={run.workflow_name} params={params} />
          <p className={RUN_HEADER_META_CLASS}>
            <RunHeaderStatus
              state={state}
              title={`Started ${formatDate(run.created_at_unix)} · updated ${formatDate(run.updated_at_unix)}`}
            >
              <span>
                {runDurationLead(state)}{' '}
                <RunDuration end={run.completed_at_unix} start={run.created_at_unix} />
              </span>
            </RunHeaderStatus>
            <span className="whitespace-nowrap">
              <RunHeaderDot />
              {runTriggerLabel(run.trigger)}
            </span>
            <span className="whitespace-nowrap">
              <RunHeaderDot />
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
      <LiveUpdatesPaused error={metadataError} onRetry={onRefresh} />
    </header>
  )
}

export const RUN_HEADER_META_CLASS =
  'mt-0.5 flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs text-muted-foreground'

export function RunHeaderTitle({ name, params }: { name: string; params: RunActionInput }) {
  return (
    <h1 className="flex min-w-0 items-baseline gap-1.5 text-[15px] leading-[22px]">
      <Link
        className="text-muted-foreground hover:text-foreground"
        params={{ owner: params.owner, repo: params.repo }}
        to="/$owner/$repo/runs"
      >
        Runs
      </Link>
      <span aria-hidden="true" className="text-muted-foreground/50">/</span>
      <span className="truncate font-semibold tracking-[-0.01em]">{name}</span>
    </h1>
  )
}

export function RunHeaderStatus({
  children,
  state,
  title,
}: {
  children: ReactNode
  state: string
  title: string | undefined
}) {
  return (
    <span
      className={cn('inline-flex items-center gap-1.5', RUN_TONE_TEXT_CLASS[runStatus(state).tone])}
      suppressHydrationWarning
      title={title}
    >
      <RunStatusIcon state={state} />
      {children}
    </span>
  )
}

export function RunHeaderDot() {
  return <span aria-hidden="true" className="mr-1.5 opacity-50">·</span>
}

export function LiveUpdatesPaused({ error, onRetry }: { error: string | null; onRetry: () => void }) {
  if (!error) return null
  return (
    <p
      className="mt-2 flex flex-wrap items-center gap-2 text-xs text-muted-foreground"
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
