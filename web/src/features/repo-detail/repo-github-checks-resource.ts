import type { GitHubChecksResponse } from '../../api/types.generated'
import { createRepoResource } from './repo-resource'

export const repoGitHubChecksResource = createRepoResource<GitHubChecksResponse>({
  invalidatedBy: ['checks'],
  maxEntries: 24,
  maxWeight: 2 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})
