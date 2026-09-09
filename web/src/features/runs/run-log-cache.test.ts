import assert from 'node:assert/strict'
import { beforeEach, test } from 'node:test'
import type { RepoRunDetail } from '@/api/types'
import { mergeStepLogPage } from './repository-run-detail-model'
import {
  canReuseRunLogs,
  completedRunLogVersion,
  EMPTY_LOG_STATE,
  runLogsResource,
  resetRunLogCache,
  runLogCacheKey,
  stepKey,
  writeRunLogCache,
  type StepLogState,
} from './run-log-cache'

const readRunLogCache = (key: string) => runLogsResource.read(key) ?? {}

const selection = { jobKey: 'test', attemptId: 'attempt-1', stepIndex: 0 }
const key = runLogCacheKey('viewer-1:repo-1:member', 'run-1')
const detail: RepoRunDetail = {
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

beforeEach(resetRunLogCache)

test('returning to completed run output reuses its loaded page without refreshing', () => {
  writeRunLogCache(key, selection, loaded)
  const revisited = readRunLogCache(key)[stepKey(selection)]!
  assert.deepEqual(revisited.logs, [log(1)])
  assert.equal(canReuseRunLogs(revisited, detail), true)
})

test('live output survives navigation and appends missing output during recovery', () => {
  const running: RepoRunDetail = {
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
