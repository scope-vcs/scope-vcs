import type { RequestComparisonMode, RequestRevisionListResponse } from '../../../api/types.generated'
import { affectsRequestKey, createRepoResource, repoResourceKey } from '../../repo-detail/repo-resource'

export const requestChangesResource = createRepoResource<RequestRevisionListResponse>({
  invalidatedBy: ['requestTimeline'],
  affectedBy: affectsRequestKey,
  maxEntries: 48,
  maxWeight: 8 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

export function requestChangesSelectionIdentity(scope: string, requestId: string, revision?: string, commit?: string, comparison?: RequestComparisonMode) {
  const query = requestChangesQuery({ comparison, commit, revision })
  return repoResourceKey(scope, requestId, query.revision_id ?? '', query.comparison, query.commit_oid ?? '')
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
) {
  const comparison = search.comparison ?? (search.commit ? 'commit' : 'request')
  return {
    comparison,
    commit_oid: comparison === 'commit' ? search.commit : undefined,
    revision_id: search.revision,
  }
}
