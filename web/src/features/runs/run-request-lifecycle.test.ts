import assert from 'node:assert/strict'
import { beforeEach, mock, test } from 'node:test'
import type { RepoRunHistoryInput, RunStepLogsInput } from '@/api/types'
import type {
  RepositoryRunDetailResponse,
  RepositoryRunHistoryPageResponse,
  RepositoryRunStepLogPageResponse,
} from '@/api/types.generated'
import { loadRunDetailSnapshot, initializeRunDetail, refreshRunDetail, runDetailResource } from './run-detail-resource'
import { loadRunPageSnapshot, runPageSnapshot, runHistoryCacheKey, initializeRunHistory, loadMoreRunHistory, refreshRunHistory, runHistoryResource } from './run-history-cache'
import { EMPTY_LOG_STATE, refreshRunLogs, refreshRunLogsAfterInFlight, runLogsResource, stepKey, writeRunLogCache } from './run-log-cache'
import { ensureRunResource } from './run-resource'
import { invalidateRepoResources } from '../repo-detail/repo-resource-invalidation'
import { resetViewerState } from '../../lib/viewer-state'

const key = 'viewer/repo/access/run'
const params = { owner: 'owner', repo: 'repo', run_id: 'run' }
const target = { jobKey: 'test', attemptId: 'attempt', stepIndex: 0 }
const detail: RepositoryRunDetailResponse = {
  run: {
    id: 'run', workflow_name: 'tests', git_oid: 'abc', trigger: 'manual',
    state: 'running', cancellation_requested: false, created_at_unix: 1,
    updated_at_unix: 2, completed_at_unix: null, can_cancel: true, can_retry: false,
  }, jobs: [],
}
const completed: RepositoryRunDetailResponse = { ...detail, run: { ...detail.run, state: 'succeeded', updated_at_unix: 3, completed_at_unix: 3 } }
function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: Error) => void
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no })
  return { promise, resolve, reject }
}
function logs(positions: number[], hasMore = false): RepositoryRunStepLogPageResponse {
  return {
    logs: positions.map((position) => ({ position, sequence: position, text: `${position}`, byte_length: 1, created_at_unix: 2 })),
    has_earlier: false, has_more: hasMore, logs_truncated: false, next_after: positions.at(-1) ?? 0,
  }
}
function historyOptions(loadHistory: (input: RepoRunHistoryInput) => Promise<RepositoryRunHistoryPageResponse | null>, historyKey = key) {
  return { key: historyKey, input: params, loadHistory, loadPage: async (input: RepoRunHistoryInput) => {
    const page = await loadHistory(input)
    return page ? { kind: 'native' as const, history: page, workflows: { workflows: [], native_runs_available: true }, workflowsError: null } : null
  } }
}
function history(ids: string[], next_cursor: string | null = null): RepositoryRunHistoryPageResponse {
  return { runs: ids.map((id) => ({ id, state: 'queued' })) as RepositoryRunHistoryPageResponse['runs'], next_cursor }
}

beforeEach(resetViewerState)

test('detail request survives navigation, deduplicates reopen and preserves newer metadata', async () => {
  initializeRunDetail(key, detail)
  const pending = deferred<RepositoryRunDetailResponse>()
  let calls = 0
  const load = () => { calls++; return pending.promise }
  const unsubscribe = runDetailResource.subscribe(key, () => {})
  const first = refreshRunDetail(key, load)
  unsubscribe()
  initializeRunDetail(key, detail)
  const reopened = refreshRunDetail(key, load)
  pending.resolve(completed)
  await Promise.all([first, reopened])
  initializeRunDetail(key, detail)
  assert.equal(calls, 1)
  assert.equal(runDetailResource.peek(key)?.detail.run.state, 'succeeded')
})

test('post-action refresh cannot settle from pre-action metadata, including a failed prior request', async () => {
  for (const fail of [false, true]) {
    runDetailResource.clear()
    initializeRunDetail(key, detail)
    const before = deferred<RepositoryRunDetailResponse>()
    const after = deferred<RepositoryRunDetailResponse>()
    let calls = 0
    const load = () => ++calls === 1 ? before.promise : after.promise
    const first = refreshRunDetail(key, load).catch(() => {})
    const action = refreshRunDetail(key, load, true)
    if (fail) before.reject(new Error('old request failed'))
    else before.resolve(detail)
    await first
    assert.equal(runDetailResource.peek(key)?.generation ?? 0, fail ? 0 : 1)
    after.resolve(completed)
    await action
    assert.equal(calls, 2)
    assert.equal(runDetailResource.peek(key)?.generation, 2)
    assert.equal(runDetailResource.peek(key)?.detail.run.state, 'succeeded')
  }
})

test('live log request survives navigation and reconciles final output after its response', async () => {
  const pending = deferred<RepositoryRunStepLogPageResponse>()
  const inputs: RunStepLogsInput[] = []
  let currentDetail = detail
  const loadLogs = (input: RunStepLogsInput) => {
    inputs.push(input)
    return inputs.length === 1 ? pending.promise : Promise.resolve(logs([2]))
  }
  const options = { key, target, detail, params, loadLogs }
  const unsubscribe = runLogsResource.subscribe(key, () => {})
  const initial = refreshRunLogs(options)
  unsubscribe()
  assert.equal(refreshRunLogs(options), initial)
  currentDetail = completed
  const final = refreshRunLogsAfterInFlight({ ...options, getDetail: () => currentDetail })
  pending.resolve(logs([1]))
  assert.equal(await final, true)
  assert.deepEqual(inputs.map(({ after }) => after), [undefined, 1])
  assert.deepEqual(runLogsResource.peek(key)?.[stepKey(target)]?.logs.map(({ position }) => position), [1, 2])
  assert.notEqual(runLogsResource.peek(key)?.[stepKey(target)]?.completedVersion, null)
})

test('failed earlier log page retries its cursor and keeps earlier/latest modes through reopening', async () => {
  writeRunLogCache(key, target, { ...EMPTY_LOG_STATE, initialized: true, logs: logs([10]).logs, nextAfter: 10 })
  const inputs: RunStepLogsInput[] = []
  const loadLogs = async (input: RunStepLogsInput) => {
    inputs.push(input)
    if (inputs.length === 1) throw new Error('disconnected')
    return logs([inputs.length === 2 ? 5 : 20])
  }
  const options = { key, target, detail: completed, params, loadLogs }
  assert.equal(await refreshRunLogs({ ...options, mode: 'earlier' }), false)
  assert.equal(runLogsResource.peek(key)?.[stepKey(target)]?.logs[0]?.position, 10)
  assert.equal(await refreshRunLogs({ ...options, mode: 'retry' }), true)
  assert.equal(runLogsResource.peek(key)?.[stepKey(target)]?.viewingEarlier, true)
  await refreshRunLogsAfterInFlight({ ...options, getDetail: () => completed })
  assert.equal(inputs.length, 2)
  await refreshRunLogs({ ...options, mode: 'latest' })
  assert.deepEqual(inputs.map(({ before }) => before), [10, 10, undefined])
  assert.equal(runLogsResource.peek(key)?.[stepKey(target)]?.viewingEarlier, false)
})

test('a viewer change prevents late log responses from restoring discarded data', async () => {
  const pending = deferred<RepositoryRunStepLogPageResponse>()
  const request = refreshRunLogs({ key, target, detail, params, loadLogs: () => pending.promise })
  resetViewerState()
  pending.resolve(logs([1]))
  assert.equal(await request, false)
  assert.equal(runLogsResource.peek(key), null)
})

test('pagination survives navigation; queued refresh reloads full depth without losing earlier rows', async () => {
  const scope = 'viewer/repo/access'
  const key = runHistoryCacheKey(scope, 'native')
  const first = history(['first'], 'older')
  initializeRunHistory(key, first)
  const older = deferred<RepositoryRunHistoryPageResponse>()
  const inputs: Array<string | undefined> = []
  const loadHistory = async ({ after }: { after?: string }) => {
    inputs.push(after)
    if (inputs.length === 1) return older.promise
    return after ? history(['updated-older']) : history(['updated-first'], 'next')
  }
  const options = historyOptions(loadHistory, key)
  const unsubscribe = runHistoryResource.subscribe(key, () => {})
  const pagination = loadMoreRunHistory(options)
  unsubscribe()
  initializeRunHistory(key, first)
  invalidateRepoResources(scope, { repo_id: 'repo', incarnation_id: 'incarnation', version: 1, kind: { RunChanged: { run_id: 'run', change: 'Created' } } })
  const refresh = refreshRunHistory(options)
  await loadMoreRunHistory(options)
  assert.deepEqual(runHistoryResource.peek(key)?.history, first)
  older.resolve(history(['older']))
  await Promise.all([pagination, refresh])
  assert.deepEqual(inputs, ['older', undefined, 'next'])
  assert.equal(runHistoryResource.peek(key)?.pageCount, 2)
  assert.deepEqual(runHistoryResource.peek(key)?.history?.runs.map(({ id }) => id), ['updated-first', 'updated-older'])
})

test('history errors retain valid rows and a retry refreshes; revoked access clears them', async () => {
  initializeRunHistory(key, history(['first'], 'older'))
  const options = historyOptions(async () => { throw new Error('offline') })
  await assert.rejects(refreshRunHistory(options), /offline/)
  assert.equal(runHistoryResource.peek(key)?.history?.runs[0]?.id, 'first')
  await refreshRunHistory(historyOptions(async () => null))
  assert.equal(runHistoryResource.peek(key)?.history, null)
  assert.equal(runHistoryResource.getSnapshot(key).error, null)
})

test('navigation and recovery share freshness, retained depth, and one in-flight history read', async (t) => {
  t.mock.timers.enable({ apis: ['Date'], now: 100_000 })
  const scope = 'viewer/repo/access'
  const historyKey = runHistoryCacheKey(scope, 'native')
  const page = { kind: 'native' as const, history: history(['first'], 'older'), workflows: { workflows: [], native_runs_available: true }, workflowsError: null }
  let pageReads = 0
  const pending = deferred<typeof page>()
  const loadPage = async () => { pageReads++; return pending.promise }
  const loadHistory = async () => history(['older'])
  const load = (signal: AbortSignal) => loadRunPageSnapshot({ key: historyKey, input: params, loadPage, loadHistory, signal })
  const navigation = ensureRunResource(runHistoryResource, historyKey, load)
  const reopened = ensureRunResource(runHistoryResource, historyKey, load)
  pending.resolve(page)
  await Promise.all([navigation, reopened])
  await ensureRunResource(runHistoryResource, historyKey, load)
  await refreshRunHistory({ key: historyKey, input: params, loadHistory, loadPage }, true)
  assert.equal(pageReads, 1)
  await loadMoreRunHistory({ key: historyKey, input: params, loadHistory })
  t.mock.timers.tick(30_001)
  const inputs: Array<string | undefined> = []
  const reload = (signal: AbortSignal) => loadRunPageSnapshot({ key: historyKey, input: params, loadPage: async () => { pageReads++; return page },
    loadHistory: async ({ after: cursor }) => { inputs.push(cursor); return cursor ? history(['updated-older']) : history(['updated-first'], 'next') }, signal })
  const refreshing = ensureRunResource(runHistoryResource, historyKey, reload)
  assert.deepEqual(runHistoryResource.peek(historyKey)?.history?.runs.map(({ id }) => id), ['first', 'older'])
  await refreshing
  assert.deepEqual(inputs, [undefined, 'next'])
  assert.equal(pageReads, 1)
  assert.equal(runHistoryResource.peek(historyKey)?.pageCount, 2)
  assert.deepEqual(runHistoryResource.peek(historyKey)?.history?.runs.map(({ id }) => id), ['updated-first', 'updated-older'])
})

test('events invalidate unmounted run resources in scope and navigation ignores obsolete responses', async () => {
  const scope = 'viewer/repo/access'
  const historyKey = runHistoryCacheKey(scope, 'native')
  const detailKey = JSON.stringify([scope, 'run'])
  runHistoryResource.seed(historyKey, runPageSnapshot({ kind: 'native', history: history(['first']), workflows: { workflows: [], native_runs_available: true }, workflowsError: null }))
  initializeRunDetail(detailKey, detail)
  const otherKey = JSON.stringify(['other-viewer/repo/access', 'run'])
  initializeRunDetail(otherKey, completed)
  const pending = deferred<RepositoryRunDetailResponse>()
  const read = ensureRunResource(runDetailResource, detailKey, (signal) => loadRunDetailSnapshot(detailKey, () => pending.promise, signal))
  await read
  invalidateRepoResources(scope, { repo_id: 'repo', incarnation_id: 'incarnation', version: 1, kind: { RunChanged: { run_id: 'run', change: 'StatusChanged' } } })
  assert.equal(runHistoryResource.getSnapshot(historyKey).stale, true)
  assert.equal(runDetailResource.getSnapshot(detailKey).stale, true)
  assert.equal(runDetailResource.getSnapshot(otherKey).stale, false)
  const old = ensureRunResource(runDetailResource, detailKey, (signal) => loadRunDetailSnapshot(detailKey, () => pending.promise, signal)).catch(() => {})
  await new Promise((resolve) => setImmediate(resolve))
  resetViewerState()
  pending.resolve(completed)
  await old
  assert.equal(runDetailResource.peek(detailKey), null)
  assert.equal(runHistoryResource.peek(historyKey), null)
})

test('a native refresh reads one history page per retained page and keeps the workflow catalog', async (t) => {
  t.mock.timers.enable({ apis: ['Date'], now: 100_000 })
  const historyKey = runHistoryCacheKey('viewer/repo/access', 'native')
  const workflows = { workflows: [], native_runs_available: true }
  runHistoryResource.seed(historyKey, runPageSnapshot({ kind: 'native', history: history(['old-native'], 'older'), workflows, workflowsError: null }))
  await loadMoreRunHistory({ key: historyKey, input: params, loadHistory: async () => history(['old-older']) })
  t.mock.timers.tick(30_001)
  const loadPage = mock.fn(async () => ({ kind: 'native' as const, history: history(['wrong']), workflows, workflowsError: null }))
  const loadHistory = mock.fn(async ({ after }: RepoRunHistoryInput) => after
    ? history(['new-older']) : history(['new-first'], 'new-cursor'))
  const options = { key: historyKey, input: params,
    loadHistory,
    loadPage,
  }
  await refreshRunHistory(options, true)
  assert.equal(loadHistory.mock.callCount(), 2)
  assert.equal(loadPage.mock.callCount(), 0)
  assert.deepEqual(runHistoryResource.peek(historyKey)?.history?.runs.map(({ id }) => id), ['new-first', 'new-older'])
  await ensureRunResource(runHistoryResource, historyKey, (signal) => loadRunPageSnapshot({ ...options, signal }))
  assert.equal(loadPage.mock.callCount(), 0)
  const page = runHistoryResource.peek(historyKey)?.page
  assert.equal(page?.kind, 'native')
  if (page?.kind === 'native') assert.deepEqual(page.workflows, workflows)
})
