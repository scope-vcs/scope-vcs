import assert from 'node:assert/strict'
import { beforeEach, test } from 'node:test'
import type { RunStepLogsInput } from '@/api/types'
import type {
  RepositoryRunDetailResponse,
  RepositoryRunHistoryPageResponse,
  RepositoryRunStepLogPageResponse,
} from '@/api/types.generated'
import { initializeRunDetail, refreshRunDetail, runDetailResource } from './run-detail-resource'
import { initializeRunHistory, loadMoreRunHistory, refreshRunHistory, resetRunHistoryCache, runHistoryResource } from './run-history-cache'
import { EMPTY_LOG_STATE, refreshRunLogs, refreshRunLogsAfterInFlight, resetRunLogCache, runLogsResource, stepKey, writeRunLogCache } from './run-log-cache'

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
function history(ids: string[], next_cursor: string | null = null): RepositoryRunHistoryPageResponse {
  return { runs: ids.map((id) => ({ id, state: 'queued' })) as RepositoryRunHistoryPageResponse['runs'], next_cursor }
}

beforeEach(() => { resetRunLogCache(); resetRunHistoryCache(); runDetailResource.clear() })

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

test('log reset prevents late responses from restoring discarded viewer data', async () => {
  const pending = deferred<RepositoryRunStepLogPageResponse>()
  const request = refreshRunLogs({ key, target, detail, params, loadLogs: () => pending.promise })
  resetRunLogCache()
  pending.resolve(logs([1]))
  assert.equal(await request, false)
  assert.equal(runLogsResource.peek(key), null)
})

test('pagination survives navigation; queued refresh reloads full depth without losing earlier rows', async () => {
  const first = history(['first'], 'older')
  initializeRunHistory(key, first)
  const older = deferred<RepositoryRunHistoryPageResponse>()
  const inputs: Array<string | undefined> = []
  const loadHistory = async ({ after }: { after?: string }) => {
    inputs.push(after)
    if (inputs.length === 1) return older.promise
    return after ? history(['updated-older']) : history(['updated-first'], 'next')
  }
  const options = { key, input: params, loadHistory }
  const unsubscribe = runHistoryResource.subscribe(key, () => {})
  const pagination = loadMoreRunHistory(options)
  unsubscribe()
  initializeRunHistory(key, first)
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
  const options = { key, input: params, loadHistory: async () => { throw new Error('offline') } }
  await assert.rejects(refreshRunHistory(options), /offline/)
  assert.equal(runHistoryResource.peek(key)?.history?.runs[0]?.id, 'first')
  await refreshRunHistory({ ...options, loadHistory: async () => null })
  assert.equal(runHistoryResource.peek(key)?.history, null)
  assert.equal(runHistoryResource.getSnapshot(key).error, null)
})
