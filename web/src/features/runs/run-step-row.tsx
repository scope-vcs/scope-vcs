import { cn } from '@/lib/utils'
import { ChevronRight } from 'lucide-react'
import type { ReactNode } from 'react'
import { RunDuration } from './run-duration'
import { RunStatusIcon } from './run-status-icon'

export function RunStepRow({
  children,
  completedAt,
  exitCode,
  name,
  onToggle,
  open,
  panelId,
  startedAt,
  state,
}: {
  children: ReactNode
  completedAt: number | null
  exitCode: number | null
  name: string
  onToggle: () => void
  open: boolean
  panelId: string
  startedAt: number | null
  state: string
}) {
  return (
    <li className="border-b border-border">
      <button
        aria-controls={panelId}
        aria-expanded={open}
        className={cn(
          'sticky top-0 z-10 grid h-9 w-full grid-cols-[0.875rem_0.875rem_minmax(0,1fr)_auto] items-center gap-2.5 bg-background px-4 text-left outline-none transition-colors hover:bg-muted focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring',
          open && 'border-b border-border bg-card',
        )}
        onClick={onToggle}
        type="button"
      >
        <ChevronRight
          aria-hidden="true"
          className={cn('size-3.5 text-muted-foreground transition-transform', open && 'rotate-90')}
        />
        <RunStatusIcon state={state} />
        <span className="truncate text-[13.5px] font-medium">{name}</span>
        <span className="flex items-center gap-2.5 text-xs text-muted-foreground">
          {exitCode !== null && exitCode !== 0 ? (
            <span className="font-mono text-danger-strong">exit {exitCode}</span>
          ) : null}
          {state === 'skipped' ? 'Skipped' : <RunDuration end={completedAt} start={startedAt} />}
        </span>
      </button>
      {open ? (
        <section aria-label={`${name} output`} className="scroll-mt-9 bg-card" id={panelId}>
          {children}
        </section>
      ) : null}
    </li>
  )
}
