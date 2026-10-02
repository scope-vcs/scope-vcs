import type { RepositoryRunWorkflowListResponse } from '../../api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'

// One entry per repository scope: the current main workflows and whether the
// repository may run them. Operator allowlist changes publish a repository
// event, which invalidates the entry for every open Runs page.
export const runWorkflowsResource = createCachedResource<RepositoryRunWorkflowListResponse>({
  maxEntries: 12,
  maxWeight: 512 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})
