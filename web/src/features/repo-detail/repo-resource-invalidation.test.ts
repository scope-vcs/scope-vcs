import { requestQueueIdentity, requestQueueResource } from '../requests/request-queue-cache'
import assert from 'node:assert/strict'
import test from 'node:test'
import type { RepoChangeEvent } from '../../api/types.generated'
import { repositoryActivityIdentity, repositoryActivityResource } from './repository-activity-resource'
import { requestActivityIdentity, requestActivityResource } from '../requests/request-activity-resource'
import { invalidateRepoResources, invalidateRepoSummaryResources } from './repo-resource-invalidation'
import { repositoryDependencyResource } from './repository-dependency-resource'
import { historyFeedResource } from '../history/history-resource-cache'
import { runWorkflowsResource } from '../runs/run-workflows-resource'
import { githubWorkflowRunsIdentity, githubWorkflowRunsResource } from '../runs/github-workflow-runs-resource'
import {
  githubWorkflowRunDetailIdentity,
  githubWorkflowRunDetailResource,
} from '../runs/github-workflow-run-detail-resource'
import type { GitHubWorkflowRunDetailResponse } from '../../api/types.generated'

const event = (kind: RepoChangeEvent['kind']): RepoChangeEvent => ({ repo_id: 'repo', incarnation_id: 'incarnation', kind, version: 2 })
function seed() {
  requestQueueResource.clear()
  const page = { requests: [], next_cursor: null, next_attention_at_unix: null }
  for (const scope of ['viewer-a', 'viewer-b']) for (const view of ['private', 'public']) requestQueueResource.write(requestQueueIdentity(scope, view), { query: 'needle', requestedQuery: 'needle', pages: { active: page, unclaimed: page, set_aside: page, done: page } })
  repositoryActivityResource.clear()
  requestActivityResource.clear()
  repositoryDependencyResource.clear()
  repositoryActivityResource.write(repositoryActivityIdentity('viewer-a', 'public'), { view: 'public', entry: null, head_oid: 'head' })
  repositoryActivityResource.write(repositoryActivityIdentity('viewer-a', 'agent'), { view: 'agent', entry: null, head_oid: 'agent-head' })
  repositoryActivityResource.write(repositoryActivityIdentity('viewer-b', 'public'), { view: 'public', entry: null, head_oid: 'other' })
  for (const id of ['one', 'two']) requestActivityResource.write(requestActivityIdentity('viewer-a', id), { events: [], through_position: 1 })
  repositoryDependencyResource.write('viewer-a', { error: null, report: null, status: 'Pending' })
  historyFeedResource.clear()
  historyFeedResource.write('viewer-a\0public\0all', { entries: [], next_cursor: null })
}

test('repository updates invalidate retained activity even when its page is unmounted', () => {
  seed()
  invalidateRepoResources('viewer-a', event({ RepositoryChanged: { reason: 'push' } }))
  assert.equal(requestQueueResource.getSnapshot(requestQueueIdentity('viewer-a', 'private')).stale, true)
  assert.equal(requestQueueResource.getSnapshot(requestQueueIdentity('viewer-a', 'public')).stale, true)
  assert.equal(requestQueueResource.getSnapshot(requestQueueIdentity('viewer-b', 'private')).stale, false)
  assert.equal(requestQueueResource.peek(requestQueueIdentity('viewer-a', 'private'))?.query, 'needle')
  assert.equal(repositoryActivityResource.getSnapshot(repositoryActivityIdentity('viewer-a', 'public')).stale, true)
  assert.equal(repositoryActivityResource.peek(repositoryActivityIdentity('viewer-a', 'public'))?.head_oid, 'head')
  assert.equal(repositoryActivityResource.getSnapshot(repositoryActivityIdentity('viewer-a', 'agent')).stale, true)
  assert.equal(repositoryActivityResource.peek(repositoryActivityIdentity('viewer-a', 'agent'))?.head_oid, 'agent-head')
  assert.equal(repositoryActivityResource.getSnapshot(repositoryActivityIdentity('viewer-b', 'public')).stale, false)
  assert.equal(requestActivityResource.getSnapshot(requestActivityIdentity('viewer-a', 'one')).stale, true)
  assert.equal(repositoryDependencyResource.getSnapshot('viewer-a').stale, true)
})

test('account deletions refresh history, whose authors carry no content version', () => {
  seed()
  invalidateRepoResources('viewer-a', event({ RepositoryChanged: { reason: 'contributor-deleted' } }))
  assert.equal(historyFeedResource.getSnapshot('viewer-a\0public\0all').stale, true)
})

test('allowlist changes refresh the retained Runs page workflows and their availability', () => {
  runWorkflowsResource.clear()
  for (const scope of ['viewer-a', 'viewer-b']) runWorkflowsResource.write(scope, { workflows: [], native_runs_available: true })
  invalidateRepoResources('viewer-a', event({ RunChanged: { run_id: 'run', change: 'StatusChanged' } }))
  assert.equal(runWorkflowsResource.getSnapshot('viewer-a').stale, false)
  invalidateRepoResources('viewer-a', event({ RepositoryChanged: { reason: 'native-runs-changed' } }))
  assert.equal(runWorkflowsResource.getSnapshot('viewer-a').stale, true)
  assert.equal(runWorkflowsResource.peek('viewer-a')?.native_runs_available, true)
  assert.equal(runWorkflowsResource.getSnapshot('viewer-b').stale, false)
})

test('request changes target one request and leave latest repository activity reusable', () => {
  seed()
  invalidateRepoResources('viewer-a', event({ RequestTimelineChanged: {
    request_id: 'one', discussion_id: 'discussion', through_position: 2, view: 'public',
  } }))
  assert.equal(requestQueueResource.getSnapshot(requestQueueIdentity('viewer-a', 'private')).stale, true)
  assert.equal(requestQueueResource.getSnapshot(requestQueueIdentity('viewer-a', 'public')).stale, true)
  assert.equal(requestQueueResource.getSnapshot(requestQueueIdentity('viewer-b', 'private')).stale, false)
  assert.equal(requestActivityResource.getSnapshot(requestActivityIdentity('viewer-a', 'one')).stale, true)
  assert.equal(requestActivityResource.getSnapshot(requestActivityIdentity('viewer-a', 'two')).stale, false)
  assert.equal(repositoryActivityResource.getSnapshot(repositoryActivityIdentity('viewer-a', 'public')).stale, false)
  assert.equal(repositoryDependencyResource.getSnapshot('viewer-a').stale, false)
})

test('dependency completion invalidates only the retained dependency report', () => {
  seed()
  invalidateRepoResources('viewer-a', event('DependenciesChanged'))
  assert.equal(repositoryDependencyResource.getSnapshot('viewer-a').stale, true)
  assert.equal(repositoryActivityResource.getSnapshot(repositoryActivityIdentity('viewer-a', 'public')).stale, false)
  assert.equal(requestActivityResource.getSnapshot(requestActivityIdentity('viewer-a', 'one')).stale, false)
})

test('run changes refresh request-owned state without invalidating repository resources', () => {
  seed()
  invalidateRepoResources('viewer-a', event({ RunChanged: { run_id: 'run', change: 'LogsAppended' } }))
  assert.equal(requestQueueResource.getSnapshot(requestQueueIdentity('viewer-a', 'private')).stale, false)
  invalidateRepoResources('viewer-a', event({ RunChanged: { run_id: 'run', change: 'StatusChanged' } }))
  assert.equal(requestQueueResource.getSnapshot(requestQueueIdentity('viewer-a', 'private')).stale, true)
  assert.equal(
    requestActivityResource.getSnapshot(requestActivityIdentity('viewer-a', 'one')).stale,
    true,
  )
  assert.equal(
    requestActivityResource.getSnapshot(requestActivityIdentity('viewer-a', 'two')).stale,
    true,
  )
  assert.equal(repositoryActivityResource.getSnapshot(repositoryActivityIdentity('viewer-a', 'public')).stale, false)
  assert.equal(repositoryDependencyResource.getSnapshot('viewer-a').stale, false)
})

test('GitHub workflow runs refresh only the retained GitHub run lists', () => {
  seed()
  githubWorkflowRunsResource.clear()
  const runs = {
    list: { actions_url: 'https://github.com/octo/repo/actions', workflow_runs: [], workflows: [], next_cursor: null },
    pages: 1,
  }
  const all = githubWorkflowRunsIdentity('viewer-a', null)
  const lint = githubWorkflowRunsIdentity('viewer-a', 'lint')
  const other = githubWorkflowRunsIdentity('viewer-b', null)
  for (const identity of [all, lint, other]) githubWorkflowRunsResource.write(identity, runs)
  invalidateRepoResources('viewer-a', event('GitHubWorkflowRunsChanged'))
  assert.equal(githubWorkflowRunsResource.getSnapshot(all).stale, true)
  assert.equal(githubWorkflowRunsResource.getSnapshot(lint).stale, true)
  assert.equal(githubWorkflowRunsResource.peek(all), runs)
  assert.equal(githubWorkflowRunsResource.getSnapshot(other).stale, false)
  assert.equal(requestQueueResource.getSnapshot(requestQueueIdentity('viewer-a', 'private')).stale, false)
  assert.equal(repositoryActivityResource.getSnapshot(repositoryActivityIdentity('viewer-a', 'public')).stale, false)

  githubWorkflowRunsResource.write(all, runs)
  invalidateRepoResources('viewer-a', event({ RepositoryChanged: { reason: 'github-connection-changed' } }))
  assert.equal(githubWorkflowRunsResource.getSnapshot(all).stale, true)
})

test('a GitHub job report refreshes only its run, keeping what the run shows', () => {
  seed()
  githubWorkflowRunsResource.clear()
  githubWorkflowRunDetailResource.clear()
  const detail = { run: {}, jobs: [], jobs_unavailable: null } as unknown as GitHubWorkflowRunDetailResponse
  const reported = githubWorkflowRunDetailIdentity('viewer-a', '7')
  const sibling = githubWorkflowRunDetailIdentity('viewer-a', '8')
  const otherScope = githubWorkflowRunDetailIdentity('viewer-b', '7')
  const list = githubWorkflowRunsIdentity('viewer-a', null)
  for (const identity of [reported, sibling, otherScope]) githubWorkflowRunDetailResource.write(identity, detail)
  githubWorkflowRunsResource.write(list, {
    list: { actions_url: 'https://github.com/octo/repo/actions', workflow_runs: [], workflows: [], next_cursor: null },
    pages: 1,
  })
  invalidateRepoResources('viewer-a', event({ GitHubWorkflowRunChanged: { github_run_id: 7 } }))
  assert.equal(githubWorkflowRunDetailResource.getSnapshot(reported).stale, true)
  assert.equal(githubWorkflowRunDetailResource.peek(reported), detail)
  assert.equal(githubWorkflowRunDetailResource.getSnapshot(sibling).stale, false)
  assert.equal(githubWorkflowRunDetailResource.getSnapshot(otherScope).stale, false)
  assert.equal(githubWorkflowRunsResource.getSnapshot(list).stale, false)

  invalidateRepoResources('viewer-a', event('GitHubWorkflowRunsChanged'))
  assert.equal(githubWorkflowRunDetailResource.getSnapshot(sibling).stale, true)
  assert.equal(githubWorkflowRunDetailResource.getSnapshot(otherScope).stale, false)
})

test('connection and lag recovery invalidate retained resources only in their scope', () => {
  for (const kind of ['Connected', 'Lagged'] as const) {
    seed()
    invalidateRepoResources('viewer-a', event(kind))
    assert.equal(requestQueueResource.getSnapshot(requestQueueIdentity('viewer-a', 'private')).stale, true)
    assert.equal(repositoryActivityResource.getSnapshot(repositoryActivityIdentity('viewer-a', 'public')).stale, true)
    assert.equal(repositoryDependencyResource.getSnapshot('viewer-a').stale, true)
    assert.equal(requestQueueResource.getSnapshot(requestQueueIdentity('viewer-b', 'private')).stale, false)
  }
})

test('a pending summary owns queue reconciliation while other resources refresh immediately', () => {
  seed()
  invalidateRepoResources('viewer-a', event('Lagged'), true)
  assert.equal(requestQueueResource.getSnapshot(requestQueueIdentity('viewer-a', 'private')).stale, false)
  assert.equal(repositoryActivityResource.getSnapshot(repositoryActivityIdentity('viewer-a', 'public')).stale, true)
  invalidateRepoSummaryResources('viewer-a')
  assert.equal(requestQueueResource.getSnapshot(requestQueueIdentity('viewer-a', 'private')).stale, true)
  assert.equal(requestQueueResource.getSnapshot(requestQueueIdentity('viewer-a', 'public')).stale, true)
  assert.equal(requestQueueResource.getSnapshot(requestQueueIdentity('viewer-b', 'private')).stale, false)
})

test('public code with unchanged version refreshes retained tree and file on repository changes', async () => {
  const { repoContentCacheKey, repoContentResource } = await import('./repo-content-cache')
  const { repoFileCacheKey, repoFileResource } = await import('./repo-file-cache')
  repoContentResource.clear()
  repoFileResource.clear()
  const identity = { scope: 'viewer-a', repoId: 'repo', view: 'public' as const, contentVersion: 0 }
  const treeKey = repoContentCacheKey(identity)
  const fileKey = repoFileCacheKey({ ...identity, path: 'README.md' })
  let loads = 0
  const loadTree = async () => { loads += 1; return { clone_remote_url: 'remote', files: [] } }
  const oldFile = { content: { kind: 'text' as const, text: 'old' }, oid: 'old', path: 'README.md', size_bytes: 3, label: 'public' as const }
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
  for (const alternate of [{ ...identity, scope: 'viewer-b' }, { ...identity, view: 'private' as const }, { ...identity, contentVersion: 1 }]) {
    assert.equal(repoContentResource.peek(repoContentCacheKey(alternate)), null)
    assert.equal(repoFileResource.peek(repoFileCacheKey({ ...alternate, path: 'README.md' })), null)
  }
  invalidateRepoResources('viewer-a')
  assert.equal(repoFileResource.getSnapshot(fileKey).stale, true)
})
