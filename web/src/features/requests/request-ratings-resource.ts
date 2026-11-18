import type { RequestRatingsResponse } from '../../api/types.generated'
import { affectsRequestKey, createRepoResource } from '../repo-detail/repo-resource'

export const requestRatingsResource = createRepoResource<RequestRatingsResponse>({
  invalidatedBy: ['requestTimeline'],
  affectedBy: affectsRequestKey,
  maxEntries: 16,
})
