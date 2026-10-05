import type { RequestChecksResponse } from '../../api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'

export const requestChecksResource = createCachedResource<RequestChecksResponse>({
  maxEntries: 16,
  maxWeight: 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export function requestChecksIdentity(scope: string, requestId: string) {
  return `${scope}\0${requestId}`
}
