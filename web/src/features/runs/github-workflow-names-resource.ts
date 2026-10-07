import type { GitHubWorkflowNamesResponse } from '@/api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'

export const githubWorkflowNamesResource = createCachedResource<GitHubWorkflowNamesResponse>({
  maxEntries: 12,
  maxWeight: 256 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})
