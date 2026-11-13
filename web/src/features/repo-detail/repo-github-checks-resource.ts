import type { GitHubChecksResponse } from '../../api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'

export const repoGitHubChecksResource = createCachedResource<GitHubChecksResponse>({
  maxEntries: 24,
  maxWeight: 2 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})
