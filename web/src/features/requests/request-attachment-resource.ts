import type {
  RequestAttachmentLimitsResponse,
  RequestAttachmentResponse,
} from '@/api/types.generated'
import { createCachedResource } from '../../lib/cached-resource'
import { createRequestAttachmentScopeTracker } from './request-attachment-scope-tracker'

export type RequestAttachmentResourceValue = {
  attachments: RequestAttachmentResponse[]
  limits: RequestAttachmentLimitsResponse
}

export const requestAttachmentResource = createCachedResource<RequestAttachmentResourceValue>({
  maxEntries: 16,
  maxWeight: 8 * 1024 * 1024,
  weightOf: (value) => JSON.stringify(value).length * 2,
})

const scopeTracker = createRequestAttachmentScopeTracker({
  maxOwners: 16,
  removePreviousScope(previous) {
    requestAttachmentResource.removeMatching(
      (identity) => identity.startsWith(`${previous}\0`),
    )
  },
})

export function activateRequestAttachmentResourceScope(accessScope: string) {
  scopeTracker.activate(accessScope)
}

export function resetRequestAttachmentResources() {
  requestAttachmentResource.clear()
  scopeTracker.reset()
}

export function requestAttachmentResourceIdentity(
  accessScope: string,
  requestId: string,
) {
  return `${accessScope}\0${requestId}`
}

export function refreshRequestAttachments(accessScope: string, requestId: string) {
  requestAttachmentResource.invalidate(
    requestAttachmentResourceIdentity(accessScope, requestId),
  )
}
