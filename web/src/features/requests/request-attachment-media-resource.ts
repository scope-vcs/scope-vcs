import type { CreateRequestAttachmentMediaGrantResponse } from '@/api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'
import { createRequestAttachmentScopeTracker } from './request-attachment-scope-tracker'

export const requestAttachmentMediaGrantResource = createCachedResource<CreateRequestAttachmentMediaGrantResponse>({
  maxEntries: 64,
})

const scopeTracker = createRequestAttachmentScopeTracker({
  maxOwners: 64,
  removePreviousScope(previous) {
    requestAttachmentMediaGrantResource.removeMatching(
      (identity) => identity.startsWith(`${previous}\0`),
    )
  },
})

export function activateRequestAttachmentMediaScope(accessScope: string) {
  scopeTracker.activate(accessScope)
}

export function resetRequestAttachmentMediaGrants() {
  requestAttachmentMediaGrantResource.clear()
  scopeTracker.reset()
}
