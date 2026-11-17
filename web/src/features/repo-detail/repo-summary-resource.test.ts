import * as assert from 'node:assert/strict'
import { afterEach, mock, test } from 'node:test'
import { HttpError, InvalidApiResponseError } from '../../api/http'
import type { RepoLiveState } from '../../api/types'
import type { RepoChangeEvent } from '../../api/types.generated'
import { STALE_BUILD_HEADER, StaleBuildError } from '../../lib/stale-build'
import { requestQueueIdentity, requestQueueResource } from '../requests/request-queue-cache'
import { invalidateRepoResources, resyncTopics, topicsOf } from './repo-resource'
import { repoResourceScope } from './repo-resource-scope'
import {
  acceptRepoSummary,
  fetchRepoSummary,
  forgetRepoSummary,
  isRetryableRepoSummaryError,
  loadRepoSummary,
  repoSummaryResource,
} from './repo-summary-resource'

const originalFetch = globalThis.fetch
afterEach(() => {
  globalThis.fetch = originalFetch
  repoSummaryResource.clear()
  requestQueueResource.clear()
})

const summary = (change_version: number, actor: 'Owner' | 'Public' = 'Owner'): RepoLiveState => ({
  api_url: 'https://scope.test',
  clerk_token_template: 'scope',
  event_stream_url: 'https://scope.test/events',
  repo: {
    id: 'owner/repo', owner_handle: 'owner', name: 'repo', description: null,
    website_url: null, git_remote_url: 'https://scope.test/repo.git',
    lifecycle_state: 'Ready', change_version, content_version: 1, open_request_count: 0,
    access: {
      actor, view: 'public', can_push: false,
      can_change_file_visibility: false, can_manage_members: false, can_delete_repo: false,
    },
    views: [{ id: 'public', name: 'Public', includes: [], readers: 'anyone' }],
  },
})
const event = (version: number, kind: RepoChangeEvent['kind'] = { RepositoryChanged: { reason: 'changed' } }): RepoChangeEvent =>
  ({ incarnation_id: 'repoi-owner-repo', kind, repo_id: 'owner/repo', version })

function retain(retained: RepoLiveState, viewerId = 'viewer') {
  repoSummaryResource.write('owner/repo', retained)
  const scope = repoResourceScope(retained.repo, viewerId)
  const page = { requests: [], next_cursor: null, next_attention_at_unix: null }
  const queue = requestQueueIdentity(scope, 'public')
  requestQueueResource.write(queue, { pages: { active: page, unclaimed: page, set_aside: page, done: page } })
  return {
    scope,
    summaryStale: () => repoSummaryResource.getSnapshot('owner/repo').stale,
    queueStale: () => requestQueueResource.getSnapshot(queue).stale,
  }
}

test('events at or below the retained version and run or discussion activity leave the summary alone', () => {
  for (const unchanged of [
    event(2),
    event(3, { RequestTimelineChanged: { view: 'public', discussion_id: 'discussion', request_id: 'request', through_position: 3 } }),
    event(3, { GitHubWorkflowRunChanged: { github_run_id: 7 } }),
    event(0, 'GitHubWorkflowRunsChanged'),
  ]) {
    const retained = retain(summary(2))
    invalidateRepoResources(retained.scope, topicsOf(unchanged, { versioned: true }))
    assert.equal(retained.summaryStale(), false, JSON.stringify(unchanged.kind))
  }
})

test('newer versions, resyncs, unversioned events, and public readers refresh the summary', () => {
  for (const changed of [
    event(6),
    event(5, 'Connected'),
    event(0, 'Lagged'),
    event(0, { RequestStateChanged: { request_id: 'request', view: 'public' } }),
    event(6, 'DependenciesChanged'),
    event(6, { RequestAttachmentChanged: { request_id: 'request', attachment_id: 'attachment', view: 'public' } }),
  ]) {
    const retained = retain(summary(5))
    invalidateRepoResources(retained.scope, topicsOf(changed, { versioned: true }))
    assert.equal(retained.summaryStale(), true, JSON.stringify(changed.kind))
    assert.equal(repoSummaryResource.peek('owner/repo')?.repo.change_version, 5)
  }
  const reader = retain(summary(5, 'Public'))
  invalidateRepoResources(reader.scope, topicsOf(event(1)))
  assert.equal(reader.summaryStale(), true)
})

test('a summary refresh owns request-list reconciliation, and events that leave the summary alone refresh the list', () => {
  const pending = retain(summary(5))
  invalidateRepoResources(pending.scope, topicsOf(event(0, 'Lagged'), { versioned: true }))
  assert.equal(pending.summaryStale(), true)
  assert.equal(pending.queueStale(), false)
  invalidateRepoResources(pending.scope, [{ topic: 'requestList' }])
  assert.equal(pending.queueStale(), true)

  for (const listOnly of [
    event(5),
    event(6, { RequestTimelineChanged: { view: 'public', discussion_id: 'discussion', request_id: 'request', through_position: 6 } }),
    event(0, 'GitHubWorkflowRunsChanged'),
  ]) {
    const settled = retain(summary(5))
    invalidateRepoResources(settled.scope, topicsOf(listOnly, { versioned: true }))
    assert.equal(settled.summaryStale(), false, JSON.stringify(listOnly.kind))
    assert.equal(settled.queueStale(), true, JSON.stringify(listOnly.kind))
  }
})

test('another repository or a resync that skips the summary leaves it retained and fresh', () => {
  const retained = retain(summary(5))
  const elsewhere = repoResourceScope({ ...summary(5).repo, id: 'owner/other' }, 'viewer')
  invalidateRepoResources(elsewhere, resyncTopics())
  assert.equal(retained.summaryStale(), false)
  invalidateRepoResources(retained.scope, resyncTopics().filter(({ topic }) => topic !== 'summary'))
  assert.equal(retained.summaryStale(), false)
  assert.equal(retained.queueStale(), true)
})

test('navigation reuses the retained summary through outages and a stale build', async () => {
  const load = mock.fn(async () => summary(1))
  const first = await loadRepoSummary('owner/repo', load)
  assert.equal(await loadRepoSummary('owner/repo', load), first)
  assert.equal(load.mock.callCount(), 1)

  for (const outage of [
    new TypeError('fetch failed'),
    new HttpError(503, { code: 'service_unavailable', message: 'Unavailable', retryable: true }),
    new StaleBuildError(),
  ]) {
    repoSummaryResource.invalidate('owner/repo')
    await repoSummaryResource.ensure('owner/repo', '', async () => { throw outage })
    assert.equal(repoSummaryResource.getSnapshot('owner/repo').error, outage)
    assert.equal(await loadRepoSummary('owner/repo', load), first)
  }
  assert.equal(load.mock.callCount(), 1)
})

test('a refused refresh makes the next navigation read again and fail with that refusal', async () => {
  await loadRepoSummary('owner/repo', async () => summary(1))
  const denied = new HttpError(403, { code: 'forbidden', message: 'Access denied', retryable: false })
  repoSummaryResource.invalidate('owner/repo')
  await repoSummaryResource.ensure('owner/repo', '', async () => { throw denied })
  await assert.rejects(loadRepoSummary('owner/repo', async () => { throw denied }), (actual) => actual === denied)
  assert.equal((await loadRepoSummary('owner/repo', async () => summary(2))).repo.change_version, 2)
})

test('a forgotten repository is read again instead of served from retention', async () => {
  await loadRepoSummary('owner/repo', async () => summary(1))
  forgetRepoSummary('owner/repo')
  const gone = new HttpError(404, { code: 'not_found', message: 'Not found', retryable: false })
  await assert.rejects(loadRepoSummary('owner/repo', async () => { throw gone }), (actual) => actual === gone)
})

test('proxy failures and unavailable results are retryable while access responses retain their status', async () => {
  globalThis.fetch = async () => new Response('proxy unavailable', { status: 502 })
  await assert.rejects(fetchRepoSummary('https://scope.test/_serverFn/id'), isRetryableRepoSummaryError)
  globalThis.fetch = async () => new Response('denied', { status: 403 })
  assert.equal((await fetchRepoSummary('https://scope.test/_serverFn/id')).status, 403)
  assert.throws(() => acceptRepoSummary({ unavailable: 'API temporarily unavailable' }), isRetryableRepoSummaryError)
  assert.equal(isRetryableRepoSummaryError(new InvalidApiResponseError('GET', '/repo', 502, 'text/html', 'content-type')), true)
  assert.equal(isRetryableRepoSummaryError(new InvalidApiResponseError('GET', '/repo', 403, 'text/html', 'content-type')), false)
})

test('a stale build stops further summary requests', async () => {
  let requests = 0
  globalThis.fetch = async () => {
    requests += 1
    return new Response('updated', { status: 409, headers: { [STALE_BUILD_HEADER]: '1' } })
  }
  await assert.rejects(fetchRepoSummary('https://scope.test/_serverFn/id'), StaleBuildError)
  await assert.rejects(fetchRepoSummary('https://scope.test/_serverFn/other'), StaleBuildError)
  assert.equal(requests, 1)
})
