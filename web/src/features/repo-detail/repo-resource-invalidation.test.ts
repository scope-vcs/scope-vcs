import { requestQueueResource } from '../requests/request-queue-cache'
import assert from 'node:assert/strict'
import test from 'node:test'
import type { RepoChangeEvent } from '../../api/types.generated'
import { repositoryActivityResource } from './repository-activity-resource'
import { requestActivityIdentity, requestActivityResource } from '../requests/request-activity-resource'
import { invalidateRepoResources } from './repo-resource-invalidation'

const event = (kind: RepoChangeEvent['kind']): RepoChangeEvent => ({ repo_id: 'repo', incarnation_id: 'incarnation', kind, version: 2 })
function seed() {
  requestQueueResource.clear()
  const page = { requests: [], next_cursor: null, next_attention_at_unix: null }
  for (const scope of ['viewer-a', 'viewer-b']) requestQueueResource.write(scope, { query: 'needle', pages: { active: page, unclaimed: page, set_aside: page } })
  repositoryActivityResource.clear()
  requestActivityResource.clear()
  repositoryActivityResource.write('viewer-a', { audience: 'public', entry: null, head_oid: 'head' })
  repositoryActivityResource.write('viewer-b', { audience: 'public', entry: null, head_oid: 'other' })
  for (const id of ['one', 'two']) requestActivityResource.write(requestActivityIdentity('viewer-a', id), { events: [], through_position: 1 })
}

test('repository updates invalidate retained activity even when its page is unmounted', () => {
  seed()
  invalidateRepoResources('viewer-a', event({ RepositoryChanged: { reason: 'push' } }))
  assert.equal(requestQueueResource.getSnapshot('viewer-a').stale, true)
  assert.equal(requestQueueResource.getSnapshot('viewer-b').stale, false)
  assert.equal(requestQueueResource.peek('viewer-a')?.query, 'needle')
  assert.equal(repositoryActivityResource.getSnapshot('viewer-a').stale, true)
  assert.equal(repositoryActivityResource.peek('viewer-a')?.head_oid, 'head')
  assert.equal(repositoryActivityResource.getSnapshot('viewer-b').stale, false)
  assert.equal(requestActivityResource.getSnapshot(requestActivityIdentity('viewer-a', 'one')).stale, true)
})

test('request changes target one request and leave latest repository activity reusable', () => {
  seed()
  invalidateRepoResources('viewer-a', event({ RequestTimelineChanged: {
    request_id: 'one', discussion_id: 'discussion', through_position: 2, audience: 'Public',
  } }))
  assert.equal(requestQueueResource.getSnapshot('viewer-a').stale, true)
  assert.equal(requestQueueResource.getSnapshot('viewer-b').stale, false)
  assert.equal(requestActivityResource.getSnapshot(requestActivityIdentity('viewer-a', 'one')).stale, true)
  assert.equal(requestActivityResource.getSnapshot(requestActivityIdentity('viewer-a', 'two')).stale, false)
  assert.equal(repositoryActivityResource.getSnapshot('viewer-a').stale, false)
})

test('recovery invalidates cached activity but ordinary connection and run events do not', () => {
  seed()
  invalidateRepoResources('viewer-a', event('Connected'))
  invalidateRepoResources('viewer-a', event({ RunChanged: { run_id: 'run', change: 'LogsAppended' } }))
  assert.equal(repositoryActivityResource.getSnapshot('viewer-a').stale, false)
  invalidateRepoResources('viewer-a', event('Lagged'))
  assert.equal(repositoryActivityResource.getSnapshot('viewer-a').stale, true)
})
