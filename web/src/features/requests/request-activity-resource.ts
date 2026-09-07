import { createCachedResource } from '../../lib/cached-resource'
import type { RequestActivityPage } from './request-discussion-types'

export const requestActivityResource = createCachedResource<RequestActivityPage>({
  maxEntries: 16,
  maxWeight: 4 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export function requestActivityIdentity(scope: string, requestId: string) {
  return `${scope}\0${requestId}`
}
