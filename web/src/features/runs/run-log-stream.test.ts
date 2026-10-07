import assert from 'node:assert/strict'
import { afterEach, beforeEach, mock, test } from 'node:test'
import { resetViewerState } from '../../lib/viewer-state'
import { appendStreamRunLog, EMPTY_LOG_STATE, runLogCacheKey, runLogsResource, startRunLogStream, stepKey, writeRunLogCache } from './run-log-cache'
import { watchRunLogs } from './run-log-stream'
import type { RunLogResponse, RunResponse } from '@/api/types.generated'

const originalFetch = globalThis.fetch
const key = runLogCacheKey('viewer:repo:member', 'run-1')
const selection = { jobKey: 'build', attemptId: 'attempt-1', stepIndex: 0 }
const log = (position: number): RunLogResponse => ({
  attempt_id: 'attempt-1', job_key: 'build', step_index: 0,
  position, sequence: position, text: `line ${position}\n`, created_at_unix: 1,
})
const finished: RunResponse = {
  id: 'run-1', repository_id: 'owner/repo', workflow_name: 'build', git_oid: 'abc',
  state: 'succeeded', cancellation_requested: false, logs_truncated: false,
  created_at_unix: 1, updated_at_unix: 2, completed_at_unix: 2,
}

beforeEach(resetViewerState)
afterEach(() => { globalThis.fetch = originalFetch })

function stream(...events: Array<[string, unknown]>) {
  const body = events.map(([name, value]) => `event: ${name}\ndata: ${JSON.stringify(value)}\n\n`).join('')
  return new Response(body, { headers: { 'content-type': 'text/event-stream' } })
}

test('open run stream resumes by position and updates the retained cache without log-page GETs', async () => {
  writeRunLogCache(key, selection, {
    ...EMPTY_LOG_STATE, initialized: true,
    logs: [{ position: 1, sequence: 1, text: 'line 1\n', byte_length: 7, created_at_unix: 1 }],
    nextAfter: 1,
  })
  const requests: string[] = []
  const authorizations: Array<string | null> = []
  globalThis.fetch = async (input, init) => {
    const url = String(input)
    requests.push(url)
    authorizations.push(new Headers(init?.headers).get('authorization'))
    return requests.length === 1
      ? stream(['log', log(2)])
      : stream(['log', log(2)], ['log', log(3)], ['status', finished])
  }
  const getToken = mock.fn(async () => 'test-token')
  const onLog = mock.fn((entry: RunLogResponse) => appendStreamRunLog(key, entry))
  await watchRunLogs({
    url: 'https://api.scope.test/v1/repos/owner/repo/runs/run-1/events',
    tokenTemplate: 'scope_api', getToken, initialCursor: 1, onLog,
    signal: AbortSignal.timeout(10_000),
  })
  assert.deepEqual(requests.map((url) => new URL(url).searchParams.get('after')), ['1', '2'])
  assert.deepEqual(authorizations, ['Bearer test-token', 'Bearer test-token'])
  assert.ok(requests.every((url) => new URL(url).pathname.endsWith('/events')))
  assert.equal(getToken.mock.callCount(), 2)
  assert.equal(onLog.mock.callCount(), 2)
  assert.deepEqual(runLogsResource.peek(key)?.[stepKey(selection)]?.logs.map(({ position }) => position), [1, 2, 3])
})

test('viewer reset aborts the stream before late output can restore the old cache', async () => {
  writeRunLogCache(key, selection, { ...EMPTY_LOG_STATE, initialized: true })
  let streamController!: ReadableStreamDefaultController<Uint8Array>
  globalThis.fetch = async () => new Response(new ReadableStream<Uint8Array>({
    start(controller) { streamController = controller },
  }), { headers: { 'content-type': 'text/event-stream' } })
  const cleanup = startRunLogStream(
    key, 'https://api.scope.test/v1/repos/owner/repo/runs/run-1/events',
    'scope_api', async () => 'test-token',
  )
  await new Promise((resolve) => setImmediate(resolve))
  resetViewerState()
  streamController.enqueue(new TextEncoder().encode(`event: log\ndata: ${JSON.stringify(log(2))}\n\n`))
  streamController.close()
  await new Promise((resolve) => setImmediate(resolve))
  assert.equal(runLogsResource.peek(key), null)
  cleanup()
})
