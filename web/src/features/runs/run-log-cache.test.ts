import assert from 'node:assert/strict'
import { beforeEach, test } from 'node:test'
import { mergeStepLogPage } from './repository-run-detail-model'
import {
  canReuseRunLogs,
  completedRunLogVersion,
  EMPTY_LOG_STATE,
  runLogsResource,
  runLogCacheKey,
  refreshRunLogs,
  stepKey,
  writeRunLogCache,
  appendStreamRunLog,
  type StepLogState,
} from './run-log-cache'
import type { RepositoryRunDetailResponse } from '@/api/types.generated'
import { resetViewerState } from '../../lib/viewer-state'

const readRunLogCache = (key: string) => runLogsResource.read(key) ?? {}

const selection = { jobKey: 'test', attemptId: 'attempt-1', stepIndex: 0 }
const key = runLogCacheKey('viewer-1:repo-1:member', 'run-1')
const detail: RepositoryRunDetailResponse = {
  run: {
    id: 'run-1', workflow_name: 'tests', git_oid: 'abc', trigger: 'manual',
    state: 'succeeded', cancellation_requested: false, created_at_unix: 1,
    updated_at_unix: 3, completed_at_unix: 3, can_cancel: false, can_retry: false,
  },
  jobs: [],
}
const log = (position: number) => ({
  position, sequence: position, text: `line ${position}\n`, byte_length: 7,
  created_at_unix: 2,
})
const loaded: StepLogState = {
  ...EMPTY_LOG_STATE,
  initialized: true,
  logs: [log(1)],
  nextAfter: 1,
  completedVersion: completedRunLogVersion(detail),
}

beforeEach(resetViewerState)

test('returning to completed run output reuses its loaded page without refreshing', () => {
  writeRunLogCache(key, selection, loaded)
  const revisited = readRunLogCache(key)[stepKey(selection)]!
  assert.deepEqual(revisited.logs, [log(1)])
  assert.equal(canReuseRunLogs(revisited, detail), true)
})

test('live output survives navigation and appends missing output during recovery', () => {
  const running: RepositoryRunDetailResponse = {
    ...detail,
    run: { ...detail.run, state: 'running', completed_at_unix: null },
  }
  writeRunLogCache(key, selection, { ...loaded, completedVersion: null })
  const revisited = readRunLogCache(key)[stepKey(selection)]!
  assert.deepEqual(revisited.logs, [log(1)])
  assert.equal(canReuseRunLogs(revisited, running), false)
  assert.equal(canReuseRunLogs(revisited, detail), false)
  const page = { logs: [log(1), log(2)], has_earlier: false }
  const merged = mergeStepLogPage(revisited, page, revisited.nextAfter)
  writeRunLogCache(key, selection, {
    ...revisited, ...merged, nextAfter: 2,
    completedVersion: completedRunLogVersion(detail),
  })
  const completed = readRunLogCache(key)[stepKey(selection)]!
  assert.deepEqual(completed.logs, [log(1), log(2)])
  assert.equal(canReuseRunLogs(completed, detail), true)
})

test('an in-flight initial page keeps newer streamed output', async () => {
  let resolvePage!: (page: { logs: ReturnType<typeof log>[]; has_earlier: boolean; has_more: boolean; logs_truncated: boolean; next_after: number }) => void
  const page = new Promise<Parameters<typeof resolvePage>[0]>((resolve) => { resolvePage = resolve })
  const request = refreshRunLogs({
    key, target: selection, detail,
    params: { owner: 'owner', repo: 'repo', run_id: 'run-1' },
    loadLogs: async () => page,
  })
  appendStreamRunLog(key, {
    attempt_id: selection.attemptId, job_key: selection.jobKey, step_index: selection.stepIndex,
    position: 2, sequence: 2, text: 'line 2\n', created_at_unix: 2,
  })
  resolvePage({ logs: [log(1)], has_earlier: false, has_more: false, logs_truncated: false, next_after: 1 })
  assert.equal(await request, true)
  assert.deepEqual(readRunLogCache(key)[stepKey(selection)]?.logs.map(({ position }) => position), [1, 2])

  let resolveLater!: typeof resolvePage
  const laterPage = new Promise<Parameters<typeof resolvePage>[0]>((resolve) => { resolveLater = resolve })
  const later = refreshRunLogs({
    key, target: selection, detail,
    params: { owner: 'owner', repo: 'repo', run_id: 'run-1' },
    loadLogs: async () => laterPage,
  })
  appendStreamRunLog(key, {
    attempt_id: selection.attemptId, job_key: selection.jobKey, step_index: selection.stepIndex,
    position: 5, sequence: 5, text: 'line 5\n', created_at_unix: 2,
  })
  resolveLater({ logs: [log(3), log(4)], has_earlier: false, has_more: false, logs_truncated: false, next_after: 4 })
  assert.equal(await later, true)
  assert.deepEqual(readRunLogCache(key)[stepKey(selection)]?.logs.map(({ position }) => position), [1, 2, 3, 4, 5])

  const earlier = readRunLogCache(key)[stepKey(selection)]!
  writeRunLogCache(key, selection, { ...earlier, viewingEarlier: true })
  let resolveLatest!: typeof resolvePage
  const latestPage = new Promise<Parameters<typeof resolvePage>[0]>((resolve) => { resolveLatest = resolve })
  const latest = refreshRunLogs({
    key, target: selection, detail,
    params: { owner: 'owner', repo: 'repo', run_id: 'run-1' },
    loadLogs: async () => latestPage,
    mode: 'latest',
  })
  appendStreamRunLog(key, {
    attempt_id: selection.attemptId, job_key: selection.jobKey, step_index: selection.stepIndex,
    position: 8, sequence: 8, text: 'line 8\n', created_at_unix: 2,
  })
  resolveLatest({ logs: [log(6), log(7)], has_earlier: true, has_more: false, logs_truncated: false, next_after: 7 })
  assert.equal(await latest, true)
  assert.deepEqual(readRunLogCache(key)[stepKey(selection)]?.logs.map(({ position }) => position), [6, 7, 8])
})

const streamed = (position: number, stepIndex = selection.stepIndex) => ({
  attempt_id: selection.attemptId, job_key: selection.jobKey, step_index: stepIndex,
  position, sequence: position, text: `line ${position}\n`, created_at_unix: 2,
})

test('stream output for a step no page has asked for is not cached', () => {
  writeRunLogCache(key, selection, loaded)
  appendStreamRunLog(key, streamed(5, 3))
  assert.deepEqual(Object.keys(readRunLogCache(key)), [stepKey(selection)])
})

test('stream output received while the first page loads is merged by position', async () => {
  let release!: () => void
  const held = new Promise<void>((resolve) => { release = resolve })
  const request = refreshRunLogs({
    key, target: selection, detail,
    params: { owner: 'owner', repo: 'repo', run_id: 'run-1' },
    loadLogs: async () => {
      await held
      return { logs: [log(1)], has_earlier: false, has_more: false, logs_truncated: false, next_after: 1 }
    },
  })
  appendStreamRunLog(key, streamed(2))
  release()
  assert.equal(await request, true)
  const state = readRunLogCache(key)[stepKey(selection)]
  assert.deepEqual(state?.logs.map(({ position }) => position), [1, 2])
  assert.equal(state?.nextAfter, 1)
})

test('streamed output never moves the page cursor past rows still to fetch', () => {
  writeRunLogCache(key, selection, loaded)
  appendStreamRunLog(key, streamed(9))
  const state = readRunLogCache(key)[stepKey(selection)]
  assert.deepEqual(state?.logs.map(({ position }) => position), [1, 9])
  assert.equal(state?.nextAfter, 1)
})

test('a changed completion revision, incomplete page or failed page still needs refresh', () => {
  const changed = { ...detail, run: { ...detail.run, updated_at_unix: 4 } }
  assert.equal(canReuseRunLogs(loaded, changed), false)
  assert.equal(canReuseRunLogs({ ...loaded, hasMore: true }, detail), false)
  assert.equal(canReuseRunLogs({ ...loaded, error: 'disconnected' }, detail), false)
  assert.equal(canReuseRunLogs(EMPTY_LOG_STATE, detail), false)
})

test('logs never cross viewer, repository, access, run, job, attempt or step identities', () => {
  writeRunLogCache(key, selection, loaded)
  for (const scope of [
    'viewer-2:repo-1:member', 'viewer-1:repo-2:member', 'viewer-1:repo-1:guest',
  ]) {
    assert.deepEqual(readRunLogCache(runLogCacheKey(scope, 'run-1')), {})
  }
  assert.deepEqual(readRunLogCache(runLogCacheKey('viewer-1:repo-1:member', 'run-2')), {})
  const retained = readRunLogCache(key)
  for (const other of [
    { ...selection, jobKey: 'build' }, { ...selection, attemptId: 'attempt-2' },
    { ...selection, stepIndex: 1 },
  ]) assert.equal(retained[stepKey(other)], undefined)
})

test('returning to an earlier window preserves its cursor and navigation state', () => {
  const earlier = { ...loaded, viewingEarlier: true, hasEarlier: true, nextAfter: 10 }
  writeRunLogCache(key, selection, earlier)
  assert.deepEqual(readRunLogCache(key)[stepKey(selection)], earlier)
})

test('retention limits steps per run and evicts old runs by access order', () => {
  for (let stepIndex = 0; stepIndex < 9; stepIndex++) {
    writeRunLogCache(key, { ...selection, stepIndex }, loaded)
  }
  assert.equal(Object.keys(readRunLogCache(key)).length, 8)
  assert.equal(readRunLogCache(key)[stepKey(selection)], undefined)
  for (let run = 0; run < 15; run++) writeRunLogCache(`run-${run}`, selection, loaded)
  readRunLogCache(key)
  writeRunLogCache('another-run', selection, loaded)
  assert.equal(Object.keys(readRunLogCache(key)).length, 8)
  assert.deepEqual(readRunLogCache('run-0'), {})
})

test('retention also bounds total log bytes', () => {
  const large = { ...loaded, logs: [{ ...log(1), byte_length: 512 * 1_024 }] }
  for (let stepIndex = 0; stepIndex < 8; stepIndex++) {
    writeRunLogCache('first', { ...selection, stepIndex }, large)
    writeRunLogCache('second', { ...selection, stepIndex }, large)
  }
  writeRunLogCache('third', selection, large)
  assert.deepEqual(readRunLogCache('first'), {})
  assert.equal(Object.keys(readRunLogCache('second')).length, 8)
})
