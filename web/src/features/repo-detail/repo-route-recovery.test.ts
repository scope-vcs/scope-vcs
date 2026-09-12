import * as assert from 'node:assert/strict'
import { afterEach, test } from 'node:test'
import { HttpError, InvalidApiResponseError } from '../../api/http'
import type { RepoLiveState } from '../../api/types'
import {
  fetchRepoRouteState,
  isRetryableRepoLoadError,
  loadRepoRouteState,
} from './repo-route-recovery'

const originalFetch = globalThis.fetch
afterEach(() => { globalThis.fetch = originalFetch })

const live: RepoLiveState = {
  clerk_token_template: 'scope',
  event_stream_url: 'https://scope.test/events',
  repo: {
    id: 'repo-1', owner_handle: 'owner', name: 'repo', description: null,
    website_url: null, git_remote_url: 'https://scope.test/repo.git',
    lifecycle_state: 'Ready', change_version: 1, open_request_count: 0,
    access: {
      actor: 'Public', can_read_private_files: false, can_push: false,
      can_change_file_visibility: false, can_apply_changes: false,
      can_manage_members: false, can_delete_repo: false,
    },
  },
}

test('background reload remains pending through transport and API outages', async () => {
  let calls = 0
  let waits = 0
  const result = await loadRepoRouteState({
    signal: new AbortController().signal,
    refresh: true,
    load: async () => {
      calls += 1
      if (calls === 1) throw new TypeError('fetch failed')
      if (calls === 2) return { unavailable: 'API temporarily unavailable' }
      return { live }
    },
    wait: async () => { waits += 1 },
  })
  assert.equal(result, live)
  assert.equal(calls, 3)
  assert.equal(waits, 2)
})

test('initial navigation fails normally and nonretryable access errors stop refresh', async () => {
  let calls = 0
  for (const [refresh, error] of [
    [false, new TypeError('offline')],
    [true, new HttpError(403, { code: 'forbidden', message: 'Access denied', retryable: false })],
    [true, new HttpError(404, { code: 'not_found', message: 'Not found', retryable: false })],
    [true, new Error('unexpected loader failure')],
  ] as const) {
    await assert.rejects(loadRepoRouteState({
      signal: new AbortController().signal,
      refresh,
      load: async () => { calls += 1; throw error },
      wait: async () => { assert.fail('terminal errors must not retry') },
    }), (actual) => actual === error)
  }
  assert.equal(calls, 4)
})

test('leaving the route cancels its delayed recovery without another request', async () => {
  const controller = new AbortController()
  let calls = 0
  const loading = loadRepoRouteState({
    signal: controller.signal,
    refresh: true,
    load: async () => { calls += 1; return { unavailable: 'offline' } },
  })
  await new Promise((resolve) => setImmediate(resolve))
  controller.abort()
  await assert.rejects(loading, { name: 'AbortError' })
  assert.equal(calls, 1)
})

test('proxy failures are retryable while access responses retain their status', async () => {
  globalThis.fetch = async () => new Response('proxy unavailable', { status: 502 })
  await assert.rejects(fetchRepoRouteState('https://scope.test/_serverFn/id'), isRetryableRepoLoadError)
  globalThis.fetch = async () => new Response('denied', { status: 403 })
  assert.equal((await fetchRepoRouteState('https://scope.test/_serverFn/id')).status, 403)
  assert.equal(isRetryableRepoLoadError(new InvalidApiResponseError('GET', '/repo', 502, 'text/html', 'content-type')), true)
  assert.equal(isRetryableRepoLoadError(new InvalidApiResponseError('GET', '/repo', 403, 'text/html', 'content-type')), false)
})
