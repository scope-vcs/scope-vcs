import type { GitHubWorkflowRunListResponse } from '@/api/types.generated'
import { createRepoResource, repoResourceKey } from '../repo-detail/repo-resource'
import { type GitHubWorkflowRunPages, mergeNextPage } from './github-workflow-run-model'

export const githubWorkflowRunsResource = createRepoResource<GitHubWorkflowRunPages>({
  invalidatedBy: ['checks'],
  maxEntries: 12,
  maxWeight: 2 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export function githubWorkflowRunsIdentity(scope: string, workflow: string | null) {
  return repoResourceKey(scope, workflow ?? '')
}

export async function loadMoreGitHubWorkflowRuns(
  identity: string,
  loadPage: (after: string, signal: AbortSignal) => Promise<GitHubWorkflowRunListResponse>,
) {
  const snapshot = githubWorkflowRunsResource.getSnapshot(identity)
  const current = snapshot.value
  const after = current?.list.next_cursor
  if (snapshot.pending || !current || !after) return
  githubWorkflowRunsResource.invalidate(identity)
  await githubWorkflowRunsResource.ensure(identity, '', async (signal) => ({
    list: mergeNextPage(current.list, await loadPage(after, signal)),
    pages: current.pages + 1,
  }))
}
