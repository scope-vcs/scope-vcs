import type { RepoRunLog, RepoRunDetail, RepoRunStepLogPage, RunStepLogsInput, RunActionInput } from '@/api/types'
import { createCachedResource } from '../../lib/cached-resource'
import { mergeStepLogPage, runCanChange, type StepSelection } from './repository-run-detail-model'

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

export const runLogsResource = createCachedResource<Record<string, StepLogState>>({
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

export function writeRunLogCache(
  key: string,
  selection: StepSelection,
  state: StepLogState,
) {
  runLogsResource.write(key, withBoundedLogStates(
    runLogsResource.peek(key) ?? {},
    stepKey(selection),
    state,
  ))
}

export function resetRunLogCache() {
  inFlight.clear()
  runLogsResource.clear()
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

function withBoundedLogStates(
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

// Different selected steps can load concurrently; requests belong to this run
// owner and survive navigation together with the observable log snapshot.
const inFlight = new Map<string, Promise<boolean>>()
export type RunLogMode = 'refresh' | 'earlier' | 'latest' | 'retry'

export function refreshRunLogs({ key, target, detail, params, loadLogs, mode = 'refresh' }: {
  key: string
  target: StepSelection
  detail: RepoRunDetail
  params: RunActionInput
  loadLogs: (input: RunStepLogsInput, signal?: AbortSignal) => Promise<RepoRunStepLogPage>
  mode?: RunLogMode
}): Promise<boolean> {
  const step = stepKey(target)
  const requestKey = JSON.stringify([key, step])
  const existing = inFlight.get(requestKey)
  if (existing) return existing
  const current = runLogsResource.peek(key)?.[step] ?? EMPTY_LOG_STATE
  if (mode === 'refresh' && current.viewingEarlier) return Promise.resolve(true)
  const before = mode === 'retry' ? current.failedPage?.before
    : mode === 'earlier' ? current.logs[0]?.position : undefined
  const after = mode === 'retry' ? current.failedPage?.after
    : mode === 'refresh' && current.initialized ? current.nextAfter : undefined
  writeRunLogCache(key, target, { ...current, error: null, loading: true })
  // A request started while running can still omit the final output.
  const completedVersion = completedRunLogVersion(detail)
  const request = Promise.resolve().then(() => loadLogs({
    ...params, after, before, attempt_id: target.attemptId, step_index: target.stepIndex,
  }, AbortSignal.timeout(15_000))).then((page) => {
    if (inFlight.get(requestKey) !== request) return false
    const previous = runLogsResource.peek(key)?.[step] ?? current
    const merged = mergeStepLogPage(previous, page, after)
    writeRunLogCache(key, target, {
      ...merged, error: null, loading: false, initialized: true,
      logsTruncated: page.logs_truncated, hasMore: page.has_more,
      viewingEarlier: before !== undefined, failedPage: null,
      nextAfter: page.next_after, completedVersion,
    })
    return true
  }, (error: unknown) => {
    if (inFlight.get(requestKey) !== request) return false
    writeRunLogCache(key, target, {
      ...(runLogsResource.peek(key)?.[step] ?? current),
      error: runErrorMessage(error), loading: false, failedPage: { after, before },
    })
    return false
  }).finally(() => {
    if (inFlight.get(requestKey) === request) inFlight.delete(requestKey)
  })
  inFlight.set(requestKey, request)
  return request
}

export async function refreshRunLogsAfterInFlight(
  options: Omit<Parameters<typeof refreshRunLogs>[0], 'detail'> & { getDetail: () => RepoRunDetail },
) {
  const step = stepKey(options.target)
  const existing = inFlight.get(JSON.stringify([options.key, step]))
  if (existing) await existing
  do {
    if (!await refreshRunLogs({ ...options, detail: options.getDetail() })) return false
    const state = runLogsResource.peek(options.key)?.[step]
    if (state?.viewingEarlier || !state?.hasMore) return true
  } while (true)
}

export function runErrorMessage(error: unknown) {
  return error instanceof Error ? error.message : 'Run operation failed.'
}
