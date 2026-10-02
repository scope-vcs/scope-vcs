import type { GitHubWorkflowRunListResponse } from '@/api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'
import { type GitHubWorkflowRunPages, mergeNextPage } from './github-workflow-run-model'

/**
 * The GitHub workflow runs a repository's Runs page lists, per repository
 * access scope and workflow filter, with every page loaded so far. Repository
 * events mark it stale, so it refreshes in place.
 */
export const githubWorkflowRunsResource = createCachedResource<GitHubWorkflowRunPages>({
  maxEntries: 12,
  maxWeight: 2 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

/** `workflow` is `null` for every workflow's runs. */
export function githubWorkflowRunsIdentity(scope: string, workflow: string | null) {
  return `${scope}\0${workflow ?? ''}`
}

/** Marks every retained list of the scope stale, whatever its filter. */
export function invalidateGitHubWorkflowRuns(scope: string) {
  githubWorkflowRunsResource.invalidateMatching((identity) => identity.startsWith(`${scope}\0`))
}

/**
 * Loads the page after the retained ones into the same entry. A refresh that
 * starts meanwhile replaces this load and reloads the pages it had.
 */
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
