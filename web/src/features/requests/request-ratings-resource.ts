import type { RequestRatingsResponse } from '../../api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'

export const requestRatingsResource = createCachedResource<RequestRatingsResponse>({
  maxEntries: 16,
})
