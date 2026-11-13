import type { RunActionInput } from '@/api/types'
import { SeparatedMeta } from '@/components/separated-meta'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import type { ReactNode } from 'react'
import { runJobTallies } from './run-job-tree-model'
import { RUN_TONE_TEXT_CLASS, RunStatusIcon } from './run-status-icon'
import { runStatus } from './run-status'

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
