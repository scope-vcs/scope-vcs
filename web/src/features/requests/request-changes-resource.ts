import type { RequestRevisionListResponse } from '../../api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'

const limits = { maxEntries: 48, maxWeight: 8 * 1024 * 1024, weightOf: (value: object) => JSON.stringify(value).length * 2 }
export const requestChangesResource = createCachedResource<RequestRevisionListResponse>(limits)

function requestChangesIdentity(scope: string, requestId: string) {
  return `${scope}\0${requestId}`
}

export function requestChangesSelectionIdentity(scope: string, requestId: string, revision?: string, commit?: string) {
  return `${requestChangesIdentity(scope, requestId)}\0${revision ?? ''}\0${commit ?? ''}`
}
