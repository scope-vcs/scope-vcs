import * as assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'
import type { RepoChangeEvent } from '@/api/types.generated'
import {
  parseRepoStreamMessage,
  reconnectDelay,
  runRepoEventStream,
  streamRepoEvents,
  takeSseMessages,
  type RepoStreamEnd,
} from './repo-event-stream'

const TEST_INCARNATION_ID = 'repoi-owner-repo'

const event = (version: number, reason = 'changed', repo_id = 'owner/repo') =>
  ({
    incarnation_id: TEST_INCARNATION_ID,
    kind: { RepositoryChanged: { reason } },
    repo_id,
    version,
  }) satisfies RepoChangeEvent
const originalFetch = globalThis.fetch

afterEach(() => {
  globalThis.fetch = originalFetch
})

test('SSE parsing returns explicit validated outcomes', () => {
  assert.deepEqual(
    parseRepoStreamMessage(
      'event: repo-change\ndata: {"repo_id":"owner/repo","incarnation_id":"repoi-owner-repo","version":2,"kind":{"RepositoryChanged":{"reason":"visibility-changed"}}}',
    ),
    { type: 'event', event: event(2, 'visibility-changed') },
  )
  assert.deepEqual(
    parseRepoStreamMessage(
      'event: repo-change\ndata: {"repo_id":"owner/repo","incarnation_id":"repoi-owner-repo","version":3,"kind":{"RunChanged":{"run_id":"run-1","change":"LogsAppended"}}}',
    ),
    {
      type: 'event',
      event: {
        incarnation_id: TEST_INCARNATION_ID,
        kind: { RunChanged: { change: 'LogsAppended', run_id: 'run-1' } },
        repo_id: 'owner/repo',
        version: 3,
      },
    },
  )
  assert.deepEqual(parseRepoStreamMessage(': keep-alive'), { type: 'ignored' })
  assert.deepEqual(parseRepoStreamMessage('event: other\ndata: {}'), { type: 'ignored' })
  assert.deepEqual(
    parseRepoStreamMessage('event: repo-change\ndata: {'),
    { type: 'protocol-error', failureClass: 'json-syntax', issuePath: '/data' },
  )
  const invalid = parseRepoStreamMessage(
    'event: repo-change\ndata: {"repo_id":"owner/repo","incarnation_id":"repoi-owner-repo","version":2,"kind":{"RequestTimelineChanged":{"request_id":"request-1"}}}',
  )
  assert.equal(invalid.type, 'protocol-error')
  assert.equal(invalid.type === 'protocol-error' ? invalid.failureClass : '', 'schema')
  assert.deepEqual(
    parseRepoStreamMessage(
      'event: error\ndata: {"code":"service_unavailable","message":"retry","retryable":true}',
    ),
    {
      type: 'stream-error',
      error: {
        code: 'service_unavailable',
        message: 'retry',
        retryable: true,
      },
    },
  )
  assert.deepEqual(takeSseMessages('event: one\n\nevent: two'), {
    messages: ['event: one'],
    rest: 'event: two',
  })
})

test('reconnect delay doubles with jitter and caps at 30 seconds', () => {
  assert.deepEqual(
    [0, 1, 2, 3, 4, 5].map((attempt) => reconnectDelay(attempt, 0)),
    [2_000, 4_000, 8_000, 16_000, 30_000, 30_000],
  )
  assert.equal(reconnectDelay(0, 1), 2_500)
  assert.equal(reconnectDelay(4, 1), 30_000)
})

test('stream connection errors use the generated public error contract', async () => {
  const connection = {
    clerk_token_template: 'scope_api',
    event_stream_url: 'https://api.scope.test/v1/repos/owner/repo/events',
  }
  globalThis.fetch = async () => new Response(JSON.stringify({
    code: 'service_unavailable',
    message: 'retry later',
    retryable: true,
  }), {
    headers: { 'content-type': 'application/json' },
    status: 503,
  })
  assert.deepEqual(
    await streamRepoEvents(
      connection,
      async () => null,
      () => assert.fail('no event expected'),
      new AbortController().signal,
    ),
    {
      type: 'stream-error',
      error: {
        code: 'service_unavailable',
        message: 'retry later',
        retryable: true,
      },
    },
  )

  globalThis.fetch = async () => new Response(JSON.stringify({ message: 'bad' }), {
    headers: { 'content-type': 'application/json' },
    status: 503,
  })
  const invalid = await streamRepoEvents(
    connection,
    async () => null,
    () => assert.fail('no event expected'),
    new AbortController().signal,
  )
  assert.equal(invalid.type, 'protocol-error')
  assert.equal(invalid.type === 'protocol-error' ? invalid.failureClass : '', 'schema')
})

test('stream decoding handles split CRLF frames before a public stream error', async () => {
  const encoder = new TextEncoder()
  const chunks = [
    'event: repo-change\r',
    '\ndata: {"repo_id":"owner/repo","incarnation_id":"repoi-owner-repo","version":2,"kind":{"RepositoryChanged":{"reason":"changed"}}}\r\n\r\n',
    'event: error\r\ndata: {"code":"forbidden","message":"access changed","retryable":false}\r\n\r\n',
  ]
  globalThis.fetch = async () => new Response(new ReadableStream({
    start(controller) {
      for (const chunk of chunks) controller.enqueue(encoder.encode(chunk))
      controller.close()
    },
  }), { headers: { 'content-type': 'text/event-stream; charset=utf-8' } })
  const delivered: RepoChangeEvent[] = []

  const outcome = await streamRepoEvents(
    {
      clerk_token_template: 'scope_api',
      event_stream_url: 'https://api.scope.test/v1/repos/owner/repo/events',
    },
    async () => 'token',
    (change) => delivered.push(change),
    new AbortController().signal,
  )

  assert.deepEqual(delivered, [event(2)])
  assert.deepEqual(outcome, {
    type: 'stream-error',
    error: { code: 'forbidden', message: 'access changed', retryable: false },
  })
})

test('protocol failures cancel response bodies before reconnecting', async () => {
  const encoder = new TextEncoder()
  let malformedBodyCanceled = false
  globalThis.fetch = async () => new Response(new ReadableStream({
    start(controller) {
      controller.enqueue(encoder.encode('event: repo-change\ndata: {\n\n'))
    },
    cancel() {
      malformedBodyCanceled = true
    },
  }), { headers: { 'content-type': 'text/event-stream' } })

  const malformed = await streamRepoEvents(
    {
      clerk_token_template: 'scope_api',
      event_stream_url: 'https://api.scope.test/v1/repos/owner/repo/events',
    },
    async () => null,
    () => assert.fail('no event expected'),
    new AbortController().signal,
  )

  assert.deepEqual(malformed, {
    type: 'protocol-error',
    failureClass: 'json-syntax',
    issuePath: '/data',
  })
  assert.equal(malformedBodyCanceled, true)

  let wrongTypeBodyCanceled = false
  globalThis.fetch = async () => new Response(new ReadableStream({
    cancel() {
      wrongTypeBodyCanceled = true
    },
  }), { headers: { 'content-type': 'text/plain' } })

  const wrongType = await streamRepoEvents(
    {
      clerk_token_template: 'scope_api',
      event_stream_url: 'https://api.scope.test/v1/repos/owner/repo/events',
    },
    async () => null,
    () => assert.fail('no event expected'),
    new AbortController().signal,
  )

  assert.deepEqual(wrongType, {
    type: 'protocol-error',
    failureClass: 'content-type',
  })
  assert.equal(wrongTypeBodyCanceled, true)
})

test('stream recovery follows retryable, protocol, and terminal outcomes', async () => {
  const outcomes: RepoStreamEnd[] = [
    { type: 'protocol-error', failureClass: 'schema', issuePath: '/kind' },
    {
      type: 'stream-error',
      error: { code: 'service_unavailable', message: 'retry', retryable: true },
    },
    {
      type: 'stream-error',
      error: { code: 'forbidden', message: 'stopped', retryable: false },
    },
  ]
  const waits: number[] = []
  const diagnostics: unknown[] = []
  let interruptions = 0
  await runRepoEventStream({
    connect: async () => outcomes.shift()!,
    onDiagnostic: (diagnostic) => diagnostics.push(diagnostic),
    onEvent: () => assert.fail('no event expected'),
    onInterrupted: () => { interruptions += 1 },
    random: () => 0,
    signal: new AbortController().signal,
    wait: async (milliseconds) => { waits.push(milliseconds) },
  })

  assert.equal(interruptions, 3)
  assert.deepEqual(waits, [2_000, 4_000])
  assert.deepEqual(diagnostics, [
    { type: 'protocol-error', failureClass: 'schema', issuePath: '/kind' },
  ])
})

test('third transport failure records once and a healthy event resets the run', async () => {
  const outcomes: RepoStreamEnd[] = [
    { type: 'transport' },
    { type: 'transport' },
    { type: 'transport' },
    { type: 'transport' },
    { type: 'transport' },
    {
      type: 'stream-error',
      error: { code: 'not_found', message: 'gone', retryable: false },
    },
  ]
  let connection = 0
  const diagnostics: unknown[] = []
  const waits: number[] = []
  await runRepoEventStream({
    connect: async (deliver) => {
      connection += 1
      if (connection === 4) deliver(event(10))
      return outcomes.shift()!
    },
    onDiagnostic: (diagnostic) => diagnostics.push(diagnostic),
    onEvent: () => {},
    onInterrupted: () => {},
    random: () => 0,
    signal: new AbortController().signal,
    wait: async (milliseconds) => { waits.push(milliseconds) },
  })

  assert.deepEqual(diagnostics, [{ type: 'transport', consecutiveFailures: 3 }])
  assert.deepEqual(waits, [2_000, 4_000, 8_000, 2_000, 4_000])
})

test('an aborted stream stops without refresh, retry, or diagnostics', async () => {
  const controller = new AbortController()
  controller.abort()
  let calls = 0
  await runRepoEventStream({
    connect: async () => { calls += 1; return { type: 'transport' } },
    onDiagnostic: () => { calls += 1 },
    onEvent: () => { calls += 1 },
    onInterrupted: () => { calls += 1 },
    signal: controller.signal,
    wait: async () => { calls += 1 },
  })
  assert.equal(calls, 0)
})
