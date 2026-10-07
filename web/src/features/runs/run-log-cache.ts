import { resourceErrorMessage } from '../../lib/use-cached-resource'
import type { RunStepLogsInput, RunActionInput } from '@/api/types'
import type {
  RepositoryRunLogResponse,
  RepositoryRunDetailResponse,
  RepositoryRunStepLogPageResponse,
  RunLogResponse,
} from '@/api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'
import { onViewerChange } from '../../lib/viewer-state'
import { mergeStepLogPage, mergeStepLogs, runCanChange, type StepSelection } from './repository-run-detail-model'
import { watchRunLogs } from './run-log-stream'

const MAX_CACHED_LOG_STEPS = 8

export type StepLogState = {
  error: string | null
  loading: boolean
  logs: RepositoryRunLogResponse[]
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

export function appendStreamRunLog(key: string, log: RunLogResponse) {
  const selection = { jobKey: log.job_key, attemptId: log.attempt_id, stepIndex: log.step_index }
  const current = runLogsResource.peek(key)?.[stepKey(selection)]
  if (!current || current.viewingEarlier) return
  if (log.position <= (current.logs.at(-1)?.position ?? 0)) return
  const entry: RepositoryRunLogResponse = {
    position: log.position,
    sequence: log.sequence,
    text: log.text,
    byte_length: new TextEncoder().encode(log.text).length,
    created_at_unix: log.created_at_unix,
  }
  const merged = mergeStepLogs(current.logs, [entry])
  runLogsResource.write(key, {
    ...runLogsResource.peek(key),
    [stepKey(selection)]: { ...current, logs: merged.logs, hasEarlier: current.hasEarlier || merged.truncated },
  })
}

function runLogStreamCursor(key: string) {
  return Math.max(0, ...Object.values(runLogsResource.peek(key) ?? {})
    .map((state) => state.logs.at(-1)?.position ?? 0))
}

export function startRunLogStream(key: string, url: string, tokenTemplate: string, getToken: (options: { template: string }) => Promise<string | null>) {
  const controller = new AbortController()
  activeStreams.add(controller)
  void watchRunLogs({
    url, tokenTemplate, getToken, initialCursor: runLogStreamCursor(key),
    onLog: (log) => {
      if (!controller.signal.aborted) appendStreamRunLog(key, log)
    },
    signal: controller.signal,
  }).finally(() => activeStreams.delete(controller))
  return () => {
    controller.abort()
    activeStreams.delete(controller)
  }
}

export function completedRunLogVersion(detail: RepositoryRunDetailResponse) {
  if (runCanChange(detail.run.state)) return null
  return JSON.stringify([
    detail.run.state,
    detail.run.updated_at_unix,
    detail.run.completed_at_unix,
  ])
}

export function canReuseRunLogs(state: StepLogState, detail: RepositoryRunDetailResponse) {
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

const inFlight = new Map<string, Promise<boolean>>()
const activeStreams = new Set<AbortController>()
onViewerChange(() => {
  inFlight.clear()
  for (const controller of activeStreams) controller.abort()
  activeStreams.clear()
})
export type RunLogMode = 'refresh' | 'earlier' | 'latest' | 'retry'

export function refreshRunLogs({ key, target, detail, params, loadLogs, mode = 'refresh' }: {
  key: string
  target: StepSelection
  detail: RepositoryRunDetailResponse
  params: RunActionInput
  loadLogs: (input: RunStepLogsInput, signal?: AbortSignal) => Promise<RepositoryRunStepLogPageResponse>
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
  writeRunLogCache(key, target, {
    ...current, error: null, loading: true,
    viewingEarlier: mode === 'latest' ? false : current.viewingEarlier,
  })
  const completedVersion = completedRunLogVersion(detail)
  const request = Promise.resolve().then(() => loadLogs({
    ...params, after, before, attempt_id: target.attemptId, step_index: target.stepIndex,
  }, AbortSignal.timeout(15_000))).then((page) => {
    if (inFlight.get(requestKey) !== request) return false
    const previous = runLogsResource.peek(key)?.[step] ?? current
    const retained = mode === 'earlier' || previous.viewingEarlier
      ? mergeStepLogPage(previous, page, after)
      : mergeFetchedRunLogs(previous, page, after)
    writeRunLogCache(key, target, {
      ...retained, error: null, loading: false, initialized: true,
      logsTruncated: page.logs_truncated, hasMore: page.has_more,
      viewingEarlier: before !== undefined, failedPage: null,
      nextAfter: page.next_after,
      completedVersion,
    })
    return true
  }, (error: unknown) => {
    if (inFlight.get(requestKey) !== request) return false
    writeRunLogCache(key, target, {
      ...(runLogsResource.peek(key)?.[step] ?? current),
      error: resourceErrorMessage(error, 'Run operation failed.'), loading: false, failedPage: { after, before },
    })
    return false
  }).finally(() => {
    if (inFlight.get(requestKey) === request) inFlight.delete(requestKey)
  })
  inFlight.set(requestKey, request)
  return request
}

function mergeFetchedRunLogs(
  previous: StepLogState,
  page: RepositoryRunStepLogPageResponse,
  after: number | undefined,
) {
  const existing = after === undefined
    ? previous.logs.filter((log) => log.position > page.next_after)
    : previous.logs
  const ordered = [...existing, ...page.logs]
    .sort((left, right) => left.position - right.position)
    .filter((log, index, logs) => log.position !== logs[index - 1]?.position)
  const merged = mergeStepLogs([], ordered)
  return {
    logs: merged.logs,
    hasEarlier: merged.truncated || (after === undefined ? page.has_earlier : previous.hasEarlier),
  }
}

export async function refreshRunLogsAfterInFlight(
  options: Omit<Parameters<typeof refreshRunLogs>[0], 'detail'> & { getDetail: () => RepositoryRunDetailResponse },
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
