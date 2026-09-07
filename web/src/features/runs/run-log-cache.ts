import type { RepoRunLog, RepoRunDetail } from '@/api/types'
import { createBoundedCache } from '../../lib/bounded-cache'
import { runCanChange, type StepSelection } from './repository-run-detail-model'

const MAX_CACHED_LOG_STEPS = 8

export type StepLogState = {
  error: string | null
  loading: boolean
  logs: RepoRunLog[]
  logsTruncated: boolean
  nextAfter: number
  initialized: boolean
  hasEarlier: boolean
  hasMore: boolean
  viewingEarlier: boolean
  failedPage: { after?: number; before?: number } | null
  completedVersion: string | null
}

export const EMPTY_LOG_STATE: StepLogState = {
  error: null,
  loading: false,
  logs: [],
  logsTruncated: false,
  nextAfter: 0,
  initialized: false,
  hasEarlier: false,
  hasMore: false,
  viewingEarlier: false,
  failedPage: null,
  completedVersion: null,
}

const entries = createBoundedCache<string, Record<string, StepLogState>>({
  maxEntries: 16,
  maxWeight: 8 * 1_024 * 1_024,
  weightOf: (states) => Object.values(states).reduce(
    (total, state) => total + state.logs.reduce(
      (bytes, log) => bytes + log.byte_length,
      0,
    ),
    0,
  ),
})

export function runLogCacheKey(scope: string, runId: string) {
  return JSON.stringify([scope, runId])
}

export function readRunLogCache(key: string) {
  return entries.get(key) ?? {}
}

export function writeRunLogCache(
  key: string,
  selection: StepSelection,
  state: StepLogState,
) {
  entries.set(key, withBoundedLogStates(
    entries.peek(key) ?? {},
    stepKey(selection),
    state,
  ))
}

export function resetRunLogCache() {
  entries.clear()
}

export function completedRunLogVersion(detail: RepoRunDetail) {
  if (runCanChange(detail.run.state)) return null
  return JSON.stringify([
    detail.run.state,
    detail.run.updated_at_unix,
    detail.run.completed_at_unix,
  ])
}

export function canReuseRunLogs(state: StepLogState, detail: RepoRunDetail) {
  const version = completedRunLogVersion(detail)
  return version !== null && state.completedVersion === version &&
    state.initialized && !state.hasMore && state.error === null
}

export function stepKey(selection: StepSelection) {
  return JSON.stringify([selection.jobKey, selection.attemptId, selection.stepIndex])
}

export function withBoundedLogStates(
  states: Record<string, StepLogState>,
  key: string,
  value: StepLogState,
) {
  const next = { ...states }
  delete next[key]
  next[key] = value
  const keys = Object.keys(next)
  for (const staleKey of keys.slice(0, -MAX_CACHED_LOG_STEPS)) {
    delete next[staleKey]
  }
  return next
}
