import type { RepositoryRunWorkflowListResponse } from '../../api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'

export const runWorkflowsResource = createCachedResource<RepositoryRunWorkflowListResponse>({
  maxEntries: 12,
  maxWeight: 512 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})
