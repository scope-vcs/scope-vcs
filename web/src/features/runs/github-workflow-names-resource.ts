import type { GitHubWorkflowNamesResponse } from '@/api/types.generated'
import { createRepoResource } from '../repo-detail/repo-resource'

export const githubWorkflowNamesResource = createRepoResource<GitHubWorkflowNamesResponse>({
  invalidatedBy: ['checks'],
  maxEntries: 12,
  maxWeight: 256 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})
