import type { RequestAutoMergeResponse } from '../../api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'

// One durable intent view per request, retained across detail-page mounts.
export const requestAutoMergeResource = createCachedResource<RequestAutoMergeResponse>({
  maxEntries: 16,
  maxWeight: 256 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export function requestAutoMergeIdentity(scope: string, requestId: string) {
  return `${scope}\0${requestId}`
}
