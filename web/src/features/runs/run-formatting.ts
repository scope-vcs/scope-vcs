import type {
  RepositoryRunState,
  RepositoryRunTrigger,
} from '@/api/types.generated'
import { runCanChange } from './repository-run-detail-model'

export function runDisplayState(run: {
  cancellation_requested: boolean
  state: RepositoryRunState
}): RepositoryRunState | 'canceling' {
  return run.cancellation_requested && runCanChange(run.state)
    ? 'canceling'
    : run.state
}

/** Elapsed seconds rendered for scanning: `44s`, `3m 04s`, `1h 12m`. */
export function formatDuration(seconds: number) {
  const safe = Math.max(0, Math.round(seconds))
  if (safe < 60) return `${safe}s`
  const minutes = Math.floor(safe / 60)
  if (minutes < 60) {
    const remaining = safe % 60
    return remaining === 0
      ? `${minutes}m`
      : `${minutes}m ${String(remaining).padStart(2, '0')}s`
  }
  const hours = Math.floor(minutes / 60)
  const remaining = minutes % 60
  return remaining === 0
    ? `${hours}h`
    : `${hours}h ${String(remaining).padStart(2, '0')}m`
}

/**
 * How long a span took, or how long it has been running. Returns null when the
 * span has not started, so callers render a placeholder instead of a fake zero.
 */
export function elapsedDuration(
  start: number | null,
  end: number | null,
  nowUnix: number,
) {
  if (start === null) return null
  return formatDuration((end ?? nowUnix) - start)
}

/** How a run started, in the words a reader would use. */
export function runTriggerLabel(trigger: RepositoryRunTrigger) {
  return trigger === 'push-main' ? 'push' : 'manual'
}
