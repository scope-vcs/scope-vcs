import type { RequestComparisonMode, RequestRevisionListResponse } from '../../api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'

const limits = { maxEntries: 48, maxWeight: 8 * 1024 * 1024, weightOf: (value: object) => JSON.stringify(value).length * 2 }
export const requestChangesResource = createCachedResource<RequestRevisionListResponse>(limits)

function requestChangesIdentity(scope: string, requestId: string) {
  return `${scope}\0${requestId}`
}

export function requestChangesSelectionIdentity(scope: string, requestId: string, revision?: string, commit?: string, comparison?: RequestComparisonMode) {
  const query = requestChangesQuery({ comparison, commit, revision })
  return `${requestChangesIdentity(scope, requestId)}\0${query.revision_id ?? ''}\0${query.comparison ?? 'default'}\0${query.commit_oid ?? ''}`
}

export function retainPinnedRequestChanges(
  scope: string,
  requestId: string,
  pin: { revision?: string; commit?: string; comparison?: RequestComparisonMode },
  revisions: RequestRevisionListResponse,
) {
  const identity = requestChangesSelectionIdentity(scope, requestId, pin.revision, pin.commit, pin.comparison)
  requestChangesResource.seed(identity, revisions)
}

export function requestChangesQuery(
  search: { comparison?: RequestComparisonMode; commit?: string; revision?: string },
  defaultComparison?: 'request' | 'push',
) {
  const comparison = search.comparison ?? (search.commit ? 'commit' : defaultComparison)
  return {
    comparison,
    commit_oid: comparison === 'commit' ? search.commit : undefined,
    revision_id: search.revision,
  }
}
