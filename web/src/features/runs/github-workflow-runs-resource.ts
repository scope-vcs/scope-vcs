import type { GitHubWorkflowRunListResponse } from '@/api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'

/**
 * The GitHub workflow runs a repository's Runs page lists, per repository
 * access scope. Repository events mark it stale, so it refreshes in place.
 */
export const githubWorkflowRunsResource = createCachedResource<GitHubWorkflowRunListResponse>({
  maxEntries: 12,
  maxWeight: 2 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})
