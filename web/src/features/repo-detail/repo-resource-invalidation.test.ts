import assert from 'node:assert/strict'
import test from 'node:test'
import type { RepoChangeEvent } from '../../api/types.generated'
import { repositoryActivityResource } from './repository-activity-resource'
import { requestActivityIdentity, requestActivityResource } from '../requests/request-activity-resource'
import { invalidateRepoResources } from './repo-resource-invalidation'

const event = (kind: RepoChangeEvent['kind']): RepoChangeEvent => ({ repo_id: 'repo', incarnation_id: 'incarnation', kind, version: 2 })
function seed() {
  repositoryActivityResource.clear()
  requestActivityResource.clear()
  repositoryActivityResource.write('viewer-a', { audience: 'public', entry: null, head_oid: 'head' })
  repositoryActivityResource.write('viewer-b', { audience: 'public', entry: null, head_oid: 'other' })
  for (const id of ['one', 'two']) requestActivityResource.write(requestActivityIdentity('viewer-a', id), { events: [], through_position: 1 })
}

test('repository updates invalidate retained activity even when its page is unmounted', () => {
  seed()
  invalidateRepoResources('viewer-a', event({ RepositoryChanged: { reason: 'push' } }))
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

test('public code with unchanged version refreshes retained tree and file on repository changes', async () => {
  const { repoContentCacheKey, repoContentResource } = await import('./repo-content-cache')
  const { repoFileCacheKey, repoFileResource } = await import('./repo-file-cache')
  repoContentResource.clear()
  repoFileResource.clear()
  const identity = { scope: 'viewer-a', repoId: 'repo', audience: 'public' as const, changeVersion: 0 }
  const treeKey = repoContentCacheKey(identity)
  const fileKey = repoFileCacheKey({ ...identity, path: 'README.md' })
  let loads = 0
  const loadTree = async () => { loads += 1; return { clone_remote_url: 'remote', files: [] } }
  const oldFile = { content: { kind: 'text' as const, text: 'old' }, oid: 'old', path: 'README.md', size_bytes: 3, visibility: 'Public' as const }
  await repoContentResource.load(treeKey, '', loadTree)
  await repoContentResource.load(treeKey, '', loadTree)
  repoFileResource.write(fileKey, oldFile)
  assert.equal(loads, 1)
  invalidateRepoResources('viewer-a', event({ RepositoryChanged: { reason: 'push' } }))
  assert.equal(repoContentResource.getSnapshot(treeKey).stale, true)
  assert.equal(repoFileResource.getSnapshot(fileKey).stale, true)
  assert.equal(repoFileResource.peek(fileKey), oldFile)
  await repoContentResource.load(treeKey, '', loadTree)
  await repoFileResource.load(fileKey, '', async () => ({ ...oldFile, oid: 'new', content: { kind: 'text', text: 'new' } }))
  assert.equal(loads, 2)
  assert.equal(repoFileResource.peek(fileKey)?.oid, 'new')
  for (const alternate of [{ ...identity, scope: 'viewer-b' }, { ...identity, audience: 'private' as const }, { ...identity, changeVersion: 1 }]) {
    assert.equal(repoContentResource.peek(repoContentCacheKey(alternate)), null)
    assert.equal(repoFileResource.peek(repoFileCacheKey({ ...alternate, path: 'README.md' })), null)
  }
  invalidateRepoResources('viewer-a')
  assert.equal(repoFileResource.getSnapshot(fileKey).stale, true)
})
